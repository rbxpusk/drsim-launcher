use crate::store::{self, FileEntry, Manifest};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;

const PARALLEL_DOWNLOADS: usize = 6;
const MAX_TRIES: u32 = 4;

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("DRFightSim-Launcher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()
        .expect("http client")
}

pub async fn fetch_manifest(c: &reqwest::Client, origins: &[String], beta: bool) -> Result<(Manifest, String), String> {
    let mut err = String::from("offline");
    for origin in origins {
        let names: &[&str] = if beta { &["manifest-beta.json", "manifest.json"] } else { &["manifest.json"] };
        for name in names {
            let url = format!("{origin}/desktop/{name}");
            let r = match c.get(&url).header("Cache-Control", "no-cache").timeout(Duration::from_secs(15)).send().await {
                Ok(r) => r,
                Err(e) => {
                    crate::logln!("manifest {url}: {}", net_err(&e));
                    if err != "not-published" && err != "blocked" {
                        err = net_err(&e);
                    }
                    break;
                }
            };
            let st = r.status().as_u16();
            if st != 200 {
                crate::logln!("manifest {url}: HTTP {st}{}", if is_challenge(&r) { " (bot check)" } else { "" });
                if st == 404 {
                    if err != "blocked" {
                        err = "not-published".into();
                    }
                    continue;
                }
                err = if is_challenge(&r) || st == 403 { "blocked".into() } else { format!("the site answered {st}") };
                break;
            }
            let b = match r.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    err = net_err(&e);
                    break;
                }
            };
            let m: Manifest = match serde_json::from_slice(&b) {
                Ok(m) => m,
                Err(_) => {
                    crate::logln!("manifest {url}: did not parse ({} bytes)", b.len());
                    err = "the update list did not read".into();
                    break;
                }
            };
            if m.v != 1 || !m.files.contains_key("index.html") {
                return Err("newer".into());
            }
            crate::logln!("manifest {url}: v{} ({} files)", m.version, m.files.len());
            return Ok((m, origin.clone()));
        }
    }
    Err(err)
}

pub fn is_challenge(r: &reqwest::Response) -> bool {
    r.headers().get("cf-mitigated").map(|v| v.as_bytes() == b"challenge").unwrap_or(false)
}

pub fn net_err(e: &reqwest::Error) -> String {
    if e.is_timeout() || e.is_connect() {
        "offline".into()
    } else {
        match e.status() {
            Some(s) => format!("HTTP {}", s.as_u16()),
            None => "network error".into(),
        }
    }
}

pub fn plan(game: &Path, installed: Option<&Manifest>, remote: &Manifest) -> Vec<(String, FileEntry)> {
    let files = game.join("files");
    remote
        .files
        .iter()
        .filter(|(rel, _)| store::safe_rel(rel))
        .filter(|(rel, f)| {
            let same = installed.and_then(|m| m.files.get(*rel)).map(|o| o.sha256 == f.sha256).unwrap_or(false);
            let size_ok = std::fs::metadata(store::under(&files, rel)).map(|m| m.len() == f.size).unwrap_or(false);
            !(same && size_ok)
        })
        .map(|(r, f)| (r.clone(), f.clone()))
        .collect()
}

pub fn split_copies(need: Vec<(String, FileEntry)>) -> (Vec<(String, FileEntry)>, Vec<(String, FileEntry, String)>) {
    let mut first: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let (mut get, mut copies) = (Vec::new(), Vec::new());
    for (rel, f) in need {
        match first.get(&f.sha256) {
            Some(src) => copies.push((rel, f.clone(), src.clone())),
            None => {
                first.insert(f.sha256.clone(), rel.clone());
                get.push((rel, f));
            }
        }
    }
    (get, copies)
}

fn place_copies(game: &Path, copies: &[(String, FileEntry, String)]) -> Result<(), String> {
    let (files, staging) = (game.join("files"), game.join("staging"));
    for (rel, f, src) in copies {
        let have = store::under(&files, rel);
        if std::fs::metadata(&have).map(|m| m.len() == f.size).unwrap_or(false) && sha256_file(&have).as_deref() == Some(f.sha256.as_str()) {
            continue;
        }
        let from = [store::under(&staging, src), store::under(&files, src)].into_iter().find(|p| std::fs::metadata(p).map(|m| m.len() == f.size).unwrap_or(false)).ok_or_else(|| format!("{rel}: its source {src} is missing"))?;
        let to = store::under(&staging, rel);
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::copy(&from, &to).map_err(|e| format!("could not install {rel}: {e}"))?;
        if std::fs::metadata(&to).map(|m| m.len()).unwrap_or(0) != f.size {
            let _ = std::fs::remove_file(&to);
            return Err(format!("{rel}: the copy did not complete"));
        }
    }
    Ok(())
}

pub fn sha256_file(p: &Path) -> Option<String> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 18];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(hex(&h.finalize()))
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[derive(Clone, Serialize)]
pub struct Progress {
    pub phase: &'static str,
    pub done: u64,
    pub total: u64,
    pub files_done: u64,
    pub files_total: u64,
    pub bps: u64,
}

pub struct Shared {
    pub done: AtomicU64,
    pub files_done: AtomicU64,
    pub cancel: Arc<AtomicBool>,
}

pub fn recover(game: &Path) {
    let next = game.join("state-next.json");
    if let Some(m) = store::load_manifest(&next) {
        let _ = commit(game, &m, store::load_manifest(&game.join("state.json")).as_ref());
    }
}

fn commit(game: &Path, m: &Manifest, old: Option<&Manifest>) -> Result<(), String> {
    let (files, staging) = (game.join("files"), game.join("staging"));
    store::write_atomic(&game.join("state-next.json"), &serde_json::to_vec(m).map_err(|e| e.to_string())?)?;
    for rel in m.files.keys().filter(|r| store::safe_rel(r)) {
        let s = store::under(&staging, rel);
        if s.is_file() {
            let d = store::under(&files, rel);
            if let Some(p) = d.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::rename(&s, &d).map_err(|e| format!("could not install {rel}: {e}"))?;
        }
    }
    if let Some(old) = old {
        for rel in old.files.keys().filter(|r| store::safe_rel(r) && !m.files.contains_key(*r)) {
            let _ = std::fs::remove_file(store::under(&files, rel));
        }
    }
    std::fs::rename(game.join("state-next.json"), game.join("state.json")).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

pub async fn run(app: AppHandle, origins: Vec<String>, game: PathBuf, remote: Manifest, cancel: Arc<AtomicBool>) -> Result<(), String> {
    std::fs::create_dir_all(game.join("files")).map_err(|e| format!("cannot write to the install folder: {e}"))?;
    recover(&game);
    let installed = store::load_manifest(&game.join("state.json"));
    let (need, copies) = split_copies(plan(&game, installed.as_ref(), &remote));
    let total: u64 = need.iter().map(|(_, f)| f.size).sum();
    let shared = Arc::new(Shared { done: AtomicU64::new(0), files_done: AtomicU64::new(0), cancel: cancel.clone() });
    let files_total = need.len() as u64;

    let tick = {
        let (app, shared) = (app.clone(), shared.clone());
        tauri::async_runtime::spawn(async move {
            let (mut last, mut bps) = (0u64, 0f64);
            loop {
                tokio::time::sleep(Duration::from_millis(125)).await;
                let d = shared.done.load(Ordering::Relaxed);
                bps = bps * 0.85 + ((d.saturating_sub(last)) as f64 * 8.0) * 0.15;
                last = d;
                let _ = app.emit_to("launcher", "sync-progress", Progress { phase: "download", done: d, total, files_done: shared.files_done.load(Ordering::Relaxed), files_total, bps: bps as u64 });
            }
        })
    };

    let c = client();
    let results: Vec<Result<(), String>> = futures_util::stream::iter(need.iter().cloned())
        .map(|(rel, f)| {
            let (c, origins, game, shared) = (c.clone(), origins.clone(), game.clone(), shared.clone());
            async move { fetch_one(&c, &origins, &game, &rel, &f, &shared).await }
        })
        .buffer_unordered(PARALLEL_DOWNLOADS)
        .collect()
        .await;
    tick.abort();
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    if let Some(e) = results.into_iter().find_map(|r| r.err()) {
        return Err(e);
    }
    let _ = app.emit_to("launcher", "sync-progress", Progress { phase: "install", done: total, total, files_done: files_total, files_total, bps: 0 });
    let g = game.clone();
    tauri::async_runtime::spawn_blocking(move || {
        place_copies(&g, &copies)?;
        commit(&g, &remote, installed.as_ref())
    })
    .await
    .map_err(|e| e.to_string())??;
    let _ = app.emit_to("launcher", "sync-progress", Progress { phase: "done", done: total, total, files_done: files_total, files_total, bps: 0 });
    Ok(())
}

async fn fetch_one(c: &reqwest::Client, origins: &[String], game: &Path, rel: &str, f: &FileEntry, shared: &Shared) -> Result<(), String> {
    if shared.cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let installed = store::under(&game.join("files"), rel);
    let staged = store::under(&game.join("staging"), rel);
    for p in [&installed, &staged] {
        if std::fs::metadata(p).map(|m| m.len() == f.size).unwrap_or(false) {
            let (p2, want) = (p.clone(), f.sha256.clone());
            let ok = tauri::async_runtime::spawn_blocking(move || sha256_file(&p2).map(|h| h == want).unwrap_or(false)).await.unwrap_or(false);
            if ok {
                if p == &installed {
                    let _ = std::fs::remove_file(&staged);
                }
                shared.done.fetch_add(f.size, Ordering::Relaxed);
                shared.files_done.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        }
    }
    let src = f.src.as_deref().filter(|s| store::safe_rel(s)).unwrap_or(rel);
    let path = src.split('/').map(encode_seg).collect::<Vec<_>>().join("/");
    let mut oi = 0usize;
    let part = PathBuf::from(format!("{}.part", staged.display()));
    if let Some(d) = part.parent() {
        tokio::fs::create_dir_all(d).await.map_err(|e| e.to_string())?;
    }
    let mut err = String::new();
    // on a hash/size mismatch (stale cdn copy) retry from pages.dev, which skips that cache
    let mut fallback: Option<&str> = None;
    for attempt in 0..MAX_TRIES {
        if shared.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(800 << attempt)).await;
        }
        let base = fallback.unwrap_or(&origins[oi.min(origins.len() - 1)]);
        let url = format!("{base}/{path}");
        match download(c, &url, &part, f, shared).await {
            Ok(()) => {
                tokio::fs::rename(&part, &staged).await.map_err(|e| e.to_string())?;
                if attempt > 0 || base != origins[0] {
                    crate::logln!("download {rel}: ok from {base} (try {})", attempt + 1);
                }
                shared.files_done.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            Err((e, counted)) => {
                shared.done.fetch_sub(counted.min(shared.done.load(Ordering::Relaxed)), Ordering::Relaxed);
                if e == "blocked" && oi + 1 < origins.len() {
                    oi += 1;
                }
                if e != "cancelled" {
                    crate::logln!("download {url}: {e} (served by {base}, try {})", attempt + 1);
                }
                if e.contains("mismatch") && base != store::PAGES && origins.iter().any(|o| o == store::PAGES) {
                    crate::logln!("download {rel}: retrying from {} after the mismatch from {base}", store::PAGES);
                    fallback = Some(store::PAGES);
                }
                err = e;
            }
        }
    }
    Err(format!("{rel}: {err}"))
}

async fn download(c: &reqwest::Client, url: &str, part: &Path, f: &FileEntry, shared: &Shared) -> Result<(), (String, u64)> {
    let mut have = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
    if have > f.size {
        let _ = tokio::fs::remove_file(part).await;
        have = 0;
    }
    let mut h = Sha256::new();
    if have > 0 {
        let p = part.to_path_buf();
        let n = have as usize;
        let pre = tokio::task::spawn_blocking(move || -> Option<Sha256> {
            let mut fh = std::fs::File::open(&p).ok()?;
            let mut h = Sha256::new();
            let mut buf = vec![0u8; 1 << 18];
            let mut left = n;
            while left > 0 {
                let k = fh.read(&mut buf[..left.min(1 << 18)]).ok()?;
                if k == 0 {
                    return None;
                }
                h.update(&buf[..k]);
                left -= k;
            }
            Some(h)
        })
        .await
        .ok()
        .flatten();
        match pre {
            Some(p) => h = p,
            None => have = 0,
        }
    }
    let mut req = c.get(url);
    if have > 0 && have < f.size {
        req = req.header("Range", format!("bytes={have}-")).header("Accept-Encoding", "identity");
    }
    let mut counted = 0u64;
    let r = req.send().await.map_err(|e| (net_err(&e), 0))?;
    let st = r.status().as_u16();
    let cache = r.headers().get("cf-cache-status").and_then(|v| v.to_str().ok()).map(|v| format!(", cf-cache-status {v}")).unwrap_or_default();
    let body_len = r.content_length();
    let mut file = if st == 206 && have > 0 {
        tokio::fs::OpenOptions::new().append(true).open(part).await.map_err(|e| (e.to_string(), 0))?
    } else if st == 200 {
        h = Sha256::new();
        have = 0;
        tokio::fs::File::create(part).await.map_err(|e| (e.to_string(), 0))?
    } else if st == 416 {
        let _ = tokio::fs::remove_file(part).await;
        return Err((format!("size mismatch: HTTP 416 for bytes {have}- of a {}-byte file{cache}", f.size), 0));
    } else if st == 403 || is_challenge(&r) {
        return Err(("blocked".into(), 0));
    } else {
        return Err((format!("HTTP {st}"), 0));
    };
    shared.done.fetch_add(have, Ordering::Relaxed);
    counted += have;
    let start = have;
    let mut body = r.bytes_stream();
    while let Some(chunk) = body.next().await {
        if shared.cancel.load(Ordering::Relaxed) {
            let _ = file.flush().await;
            return Err(("cancelled".into(), counted));
        }
        let chunk = chunk.map_err(|e| (net_err(&e), counted))?;
        if have + chunk.len() as u64 > f.size {
            drop(file);
            let _ = tokio::fs::remove_file(part).await;
            return Err((format!("size mismatch: the server sent more than the list's {} bytes{cache}", f.size), counted));
        }
        file.write_all(&chunk).await.map_err(|e| (e.to_string(), counted))?;
        h.update(&chunk);
        have += chunk.len() as u64;
        counted += chunk.len() as u64;
        shared.done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    file.flush().await.map_err(|e| (e.to_string(), counted))?;
    drop(file);
    if have != f.size {
        // fewer bytes than announced means a cut transfer we can resume; otherwise it is the wrong file
        let cut = body_len.map(|n| start + n > have).unwrap_or(false);
        if cut {
            return Err(("download cut short".into(), counted));
        }
        let _ = tokio::fs::remove_file(part).await;
        return Err((format!("size mismatch: got {have} bytes, the list says {}{cache}", f.size), counted));
    }
    if hex(&h.finalize()) != f.sha256 {
        let _ = tokio::fs::remove_file(part).await;
        return Err((format!("checksum mismatch{cache}"), counted));
    }
    Ok(())
}

pub fn verify(app: &AppHandle, game: &Path) -> Result<(u64, Vec<String>), String> {
    let m = store::load_manifest(&game.join("state.json")).ok_or("the game is not installed")?;
    let files = game.join("files");
    let total: u64 = m.files.values().map(|f| f.size).sum();
    let (mut done, mut n, mut bad) = (0u64, 0u64, Vec::new());
    let mut last = std::time::Instant::now();
    for (rel, f) in m.files.iter().filter(|(r, _)| store::safe_rel(r)) {
        let p = store::under(&files, rel);
        let ok = std::fs::metadata(&p).map(|x| x.len() == f.size).unwrap_or(false) && sha256_file(&p).map(|h| h == f.sha256).unwrap_or(false);
        if !ok {
            let _ = std::fs::remove_file(&p);
            bad.push(rel.clone());
        }
        done += f.size;
        n += 1;
        if last.elapsed() > Duration::from_millis(120) {
            last = std::time::Instant::now();
            let _ = app.emit_to("launcher", "sync-progress", Progress { phase: "verify", done, total, files_done: n, files_total: m.files.len() as u64, bps: 0 });
        }
    }
    let _ = app.emit_to("launcher", "sync-progress", Progress { phase: "done", done: total, total, files_done: n, files_total: n, bps: 0 });
    crate::logln!("verify: {n} files, {} bad{}", bad.len(), if bad.is_empty() { String::new() } else { format!(": {}", bad.iter().take(8).cloned().collect::<Vec<_>>().join(", ")) });
    Ok((n, bad))
}

fn encode_seg(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}
