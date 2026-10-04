use crate::store::{self, Manifest, SITE};
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tauri::http::{Request, Response, StatusCode};

pub struct Served {
    pub dir: PathBuf,
    pub files: HashSet<String>,
    pub headers: BTreeMap<String, String>,
}

#[derive(Default)]
pub struct ServedState(pub RwLock<Option<Arc<Served>>>);

pub fn load(game: &std::path::Path) -> Option<Served> {
    let m: Manifest = store::load_manifest(&game.join("state.json"))?;
    Some(Served { dir: game.join("files"), files: m.files.keys().cloned().collect(), headers: m.headers })
}

fn ctype(rel: &str) -> &'static str {
    let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json; charset=utf-8",
        "lrc" | "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn plain(sv: Option<&Served>, code: u16, body: &str) -> Response<Vec<u8>> {
    let mut b = Response::builder().status(code).header("Content-Type", "text/plain; charset=utf-8").header("Cache-Control", "no-store");
    if let Some(sv) = sv {
        for (k, v) in &sv.headers {
            b = b.header(k.as_str(), v.as_str());
        }
    }
    b.body(body.as_bytes().to_vec()).unwrap()
}

fn decode(p: &str) -> Option<String> {
    let b = p.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub fn handle(sv: Option<Arc<Served>>, req: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let Some(sv) = sv else { return plain(None, 503, "The game is not installed yet.\n") };
    let m = req.method().as_str();
    if m != "GET" && m != "HEAD" {
        return plain(Some(&sv), 405, "405 Method Not Allowed\n");
    }
    let raw = req.uri().path();
    if raw.len() > 1024 || raw.contains('\\') || raw.contains("%00") || raw.to_ascii_lowercase().contains("%2f") || raw.to_ascii_lowercase().contains("%5c") {
        return not_found(&sv, req);
    }
    let Some(p) = decode(raw) else { return plain(Some(&sv), 400, "400 Bad Request\n") };
    let mut rel = if p == "/" || p.is_empty() { "index.html".to_string() } else { p.trim_start_matches('/').to_string() };
    if !store::safe_rel(&rel) {
        return not_found(&sv, req);
    }
    let last = rel.rsplit('/').next().unwrap_or("");
    if !sv.files.contains(&rel) && !last.contains('.') && sv.files.contains(&(rel.clone() + ".html")) {
        rel.push_str(".html");
    }
    if !sv.files.contains(&rel) {
        return not_found(&sv, req);
    }
    let path = store::under(&sv.dir, &rel);
    let Ok(mut f) = std::fs::File::open(&path) else { return not_found(&sv, req) };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut b = Response::builder().header("Content-Type", ctype(&rel)).header("Cache-Control", "no-cache").header("Accept-Ranges", "bytes");
    for (k, v) in &sv.headers {
        b = b.header(k.as_str(), v.as_str());
    }
    let head = m == "HEAD";
    if let Some(r) = req.headers().get("range").and_then(|v| v.to_str().ok()) {
        let r = r.trim();
        let bad = || Response::builder().status(416).header("Content-Range", format!("bytes */{size}")).body(Vec::new()).unwrap();
        let Some(spec) = r.strip_prefix("bytes=") else { return bad() };
        let Some((a, z)) = spec.split_once('-') else { return bad() };
        let (start, end) = match (a.parse::<u64>().ok(), z.parse::<u64>().ok()) {
            (None, Some(n)) if a.is_empty() => (size.saturating_sub(n), size.saturating_sub(1)),
            (Some(s), None) if z.is_empty() => (s, size.saturating_sub(1)),
            (Some(s), Some(e)) => (s, e.min(size.saturating_sub(1))),
            _ => return bad(),
        };
        if size == 0 || start > end || start >= size {
            return bad();
        }
        let n = (end - start + 1) as usize;
        let mut buf = vec![0u8; if head { 0 } else { n }];
        if !head && (f.seek(SeekFrom::Start(start)).is_err() || f.read_exact(&mut buf).is_err()) {
            return plain(Some(&sv), 500, "500 Internal Server Error\n");
        }
        return b.status(206).header("Content-Range", format!("bytes {start}-{end}/{size}")).header("Content-Length", n.to_string()).body(buf).unwrap();
    }
    let mut buf = Vec::with_capacity(if head { 0 } else { size as usize });
    if !head && f.read_to_end(&mut buf).is_err() {
        return plain(Some(&sv), 500, "500 Internal Server Error\n");
    }
    b.status(StatusCode::OK).header("Content-Length", size.to_string()).body(buf).unwrap()
}

fn not_found(sv: &Served, req: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    crate::logln!("game: 404 {}", req.uri().path().chars().take(200).collect::<String>());
    let wants_html = req.headers().get("accept").and_then(|v| v.to_str().ok()).map(|a| a.contains("text/html")).unwrap_or(false);
    if wants_html && sv.files.contains("404.html") {
        if let Ok(body) = std::fs::read(sv.dir.join("404.html")) {
            let mut b = Response::builder().status(404).header("Content-Type", ctype("404.html")).header("Cache-Control", "no-cache");
            for (k, v) in &sv.headers {
                b = b.header(k.as_str(), v.as_str());
            }
            return b.body(body).unwrap();
        }
    }
    plain(Some(sv), 404, "404 Not Found\n")
}

pub fn is_api(req: &Request<Vec<u8>>) -> bool {
    matches!(req.uri().path(), "/api/stats" | "/api/report")
}

pub async fn forward(c: reqwest::Client, req: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let fail = || Response::builder().status(503).header("Content-Type", "text/plain; charset=utf-8").header("Cache-Control", "no-store").body(b"offline\n".to_vec()).unwrap();
    let url = format!("{SITE}{}", req.uri().path());
    let rb = match req.method().as_str() {
        "POST" => {
            if req.body().len() > 64 * 1024 {
                return fail();
            }
            let ct = req.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("text/plain").to_string();
            c.post(&url).header("Content-Type", ct).header("Origin", SITE).body(req.body().clone())
        }
        "GET" => c.get(&url),
        _ => return fail(),
    };
    match rb.timeout(Duration::from_secs(8)).send().await {
        Ok(r) => {
            let st = r.status().as_u16();
            let ct = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("text/plain; charset=utf-8").to_string();
            let body = r.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
            Response::builder().status(st).header("Content-Type", ct).header("Cache-Control", "no-store").body(body).unwrap()
        }
        Err(_) => fail(),
    }
}

pub fn offline() -> Response<Vec<u8>> {
    Response::builder().status(503).header("Content-Type", "text/plain; charset=utf-8").header("Cache-Control", "no-store").body(b"offline\n".to_vec()).unwrap()
}
