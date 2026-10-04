mod logs;
mod perf;
mod presence;
mod serve;
mod store;
mod sync;

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::UpdaterExt;

const DISCORD_INVITE: &str = "https://discord.gg/fWTAXYc4wR";

#[derive(Default)]
struct AppState {
    remote: Mutex<Option<(store::Manifest, String)>>,
    syncing: Mutex<Option<Arc<AtomicBool>>>,
    launcher_update: Mutex<Option<tauri_plugin_updater::Update>>,
    quit_after_game: AtomicBool,
    game_opened_ms: Mutex<i64>,
    started_hidden: AtomicBool,
    game_shown: AtomicBool,
    last_presence: Mutex<Option<(String, String, String)>>,
    fighting: AtomicBool,
}

fn reload_served(app: &AppHandle) {
    let s = store::load_settings(app);
    let sv = serve::load(&store::game_dir(app, &s)).map(Arc::new);
    *app.state::<serve::ServedState>().0.write().unwrap() = sv;
}

fn show_launcher(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("launcher") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[derive(Serialize)]
struct LauncherState {
    launcher_version: String,
    webview_version: String,
    os: String,
    portable: bool,
    installed: Option<String>,
    installed_title: String,
    installed_bytes: u64,
    install_dir: String,
    free_bytes: Option<u64>,
    log_dir: String,
    start_hidden: bool,
    first_run: bool,
    desktop_shortcut: Option<bool>,
    game_open: bool,
    settings: store::Settings,
}

#[tauri::command]
fn launcher_state(app: AppHandle, st: State<'_, AppState>) -> LauncherState {
    let s = store::load_settings(&app);
    let game = store::game_dir(&app, &s);
    sync::recover(&game);
    let m = store::load_manifest(&game.join("state.json"));
    LauncherState {
        launcher_version: app.package_info().version.to_string(),
        webview_version: tauri::webview_version().unwrap_or_default(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        portable: store::is_portable(),
        installed: m.as_ref().map(|m| m.version.clone()),
        installed_title: m.as_ref().map(|m| m.title.clone()).unwrap_or_default(),
        installed_bytes: m.as_ref().map(|m| m.bytes).unwrap_or(0),
        install_dir: game.display().to_string(),
        free_bytes: store::free_space(&game),
        log_dir: logs::dir().map(|d| d.display().to_string()).unwrap_or_default(),
        start_hidden: st.started_hidden.load(Ordering::Relaxed),
        first_run: std::env::args().any(|a| a == "--first-run"),
        desktop_shortcut: shortcut_paths(&app).map(|(l, _)| l.is_file()).filter(|on| *on || shortcut_paths(&app).map(|(_, k)| k.is_file()).unwrap_or(false)),
        game_open: st.game_shown.load(Ordering::Relaxed),
        settings: s,
    }
}

#[derive(Serialize)]
struct Check {
    online: bool,
    error: String,
    version: String,
    title: String,
    date: String,
    need_files: u64,
    need_bytes: u64,
    staged_bytes: u64,
    total_bytes: u64,
    launcher_update: Option<String>,
}

#[tauri::command]
async fn check(app: AppHandle, st: State<'_, AppState>) -> Result<Check, String> {
    let s = store::load_settings(&app);
    let game = store::game_dir(&app, &s);
    let mut out = Check { online: false, error: String::new(), version: String::new(), title: String::new(), date: String::new(), need_files: 0, need_bytes: 0, staged_bytes: 0, total_bytes: 0, launcher_update: None };
    if s.offline {
        out.error = "offline-mode".into();
        return Ok(out);
    }
    let c = sync::client();
    match sync::fetch_manifest(&c, &store::origins(), s.channel == "beta").await {
        Ok((m, origin)) => {
            let (g, m2) = (game.clone(), m.clone());
            let need = tauri::async_runtime::spawn_blocking(move || {
                sync::recover(&g);
                sync::plan(&g, store::load_manifest(&g.join("state.json")).as_ref(), &m2)
            })
            .await
            .map_err(|e| e.to_string())?;
            let staging = game.join("staging");
            out.online = true;
            out.version = m.version.clone();
            out.title = m.title.clone();
            out.date = m.date.clone();
            out.total_bytes = m.bytes;
            let (need, _) = sync::split_copies(need);
            out.need_files = need.len() as u64;
            out.staged_bytes = need.iter().filter(|(r, f)| std::fs::metadata(store::under(&staging, r)).map(|x| x.len() == f.size).unwrap_or(false)).map(|(_, f)| f.size).sum();
            out.need_bytes = need.iter().map(|(_, f)| f.size).sum::<u64>() - out.staged_bytes;
            logln!("check: v{} from {origin}; {} files / {} bytes to download", m.version, out.need_files, out.need_bytes);
            *st.remote.lock().unwrap() = Some((m, origin));
        }
        Err(e) => {
            logln!("check: {e}");
            out.online = e == "not-published" || e == "blocked";
            out.error = e;
        }
    }
    if out.online {
        if let Ok(u) = app.updater() {
            match u.check().await {
                Ok(Some(up)) => {
                    logln!("launcher update available: v{}", up.version);
                    out.launcher_update = Some(up.version.clone());
                    *st.launcher_update.lock().unwrap() = Some(up);
                }
                Ok(None) => {}
                Err(e) => logln!("launcher update check: {e}"),
            }
        }
    }
    Ok(out)
}

#[tauri::command]
async fn sync(app: AppHandle, st: State<'_, AppState>) -> Result<(), String> {
    drop_prewarm_wait(&app).await;
    if app.get_webview_window("game").is_some() {
        return Err("close the game first".into());
    }
    let (remote, origin) = st.remote.lock().unwrap().clone().ok_or("check for updates first")?;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut g = st.syncing.lock().unwrap();
        if g.is_some() {
            return Err("already downloading".into());
        }
        *g = Some(cancel.clone());
    }
    let s = store::load_settings(&app);
    let mut origins = store::origins();
    origins.retain(|o| o != &origin);
    origins.insert(0, origin);
    logln!("sync: v{} into {}", remote.version, store::game_dir(&app, &s).display());
    let r = sync::run(app.clone(), origins, store::game_dir(&app, &s), remote, cancel).await;
    match &r {
        Ok(()) => logln!("sync: done"),
        Err(e) => logln!("sync: stopped: {e}"),
    }
    *st.syncing.lock().unwrap() = None;
    reload_served(&app);
    r
}

#[derive(Serialize)]
struct Verified {
    checked: u64,
    bad: u64,
}

#[tauri::command]
async fn verify(app: AppHandle, st: State<'_, AppState>) -> Result<Verified, String> {
    drop_prewarm_wait(&app).await;
    if app.get_webview_window("game").is_some() {
        return Err("close the game first".into());
    }
    {
        let mut g = st.syncing.lock().unwrap();
        if g.is_some() {
            return Err("a download is running".into());
        }
        *g = Some(Arc::new(AtomicBool::new(false)));
    }
    let s = store::load_settings(&app);
    let (a, game) = (app.clone(), store::game_dir(&app, &s));
    let r = tauri::async_runtime::spawn_blocking(move || sync::verify(&a, &game)).await.map_err(|e| e.to_string());
    *st.syncing.lock().unwrap() = None;
    let (checked, bad) = r??;
    Ok(Verified { checked, bad: bad.len() as u64 })
}

#[tauri::command]
fn cancel_sync(st: State<'_, AppState>) {
    if let Some(c) = st.syncing.lock().unwrap().as_ref() {
        c.store(true, Ordering::Relaxed);
    }
}

fn game_init_script(app: &AppHandle, s: &store::Settings) -> String {
    let info = serde_json::json!({ "launcher": app.package_info().version.to_string(), "perf": s.perf, "channel": s.channel, "offline": s.offline });
    let mut js = format!("(function(){{if(location.hostname!=='drsim.localhost'&&location.protocol!=='drsim:')return;try{{Object.defineProperty(window,'__drDesktop',{{value:Object.freeze({info})}})}}catch(e){{}}");
    if s.volume_dirty {
        js += &format!(
            "try{{var k='dr-sim-cfg',c=JSON.parse(localStorage.getItem(k)||'{{}}')||{{}};c.musicvol={};c.sfxvol={};localStorage.setItem(k,JSON.stringify(c))}}catch(e){{}}",
            (s.music.min(100) as f64) / 100.0,
            (s.sfx.min(100) as f64) / 100.0
        );
    }
    js + "})();"
}

// async so the window is not built on the main thread the command holds
#[tauri::command]
async fn play(app: AppHandle) -> Result<(), String> {
    open_game(&app, true)
}

// boot the game hidden behind the launcher so play only has to show it
#[tauri::command]
async fn prewarm(app: AppHandle) -> Result<(), String> {
    if app.get_webview_window("game").is_some() {
        return Ok(());
    }
    open_game(&app, false)
}

fn game_geometry(app: &AppHandle, s: &store::Settings) -> (f64, f64, Option<(f64, f64)>) {
    let mon = app.get_webview_window("launcher").and_then(|w| w.current_monitor().ok().flatten()).or_else(|| app.primary_monitor().ok().flatten());
    let (mw, mh, mx, my) = mon
        .as_ref()
        .map(|m| {
            let sz = m.size().to_logical::<f64>(m.scale_factor());
            let p = m.position().to_logical::<f64>(m.scale_factor());
            (sz.width, sz.height, p.x, p.y)
        })
        .unwrap_or((1280.0, 960.0, 0.0, 0.0));
    if s.window_mode == "borderless" {
        return (mw, mh, Some((mx, my)));
    }
    let fit = ((mw * 0.92) / 640.0).min((mh * 0.86) / 480.0);
    let k = if s.window_scale == 0 { fit.max(0.5) } else { (s.window_scale as f64).min(fit.floor().max(1.0)) };
    (640.0 * k, 480.0 * k, None)
}

fn show_game(app: &AppHandle, w: &tauri::WebviewWindow) {
    let s = store::load_settings(app);
    let (gw, gh, pos) = game_geometry(app, &s);
    let _ = w.set_fullscreen(false);
    let _ = w.set_decorations(s.window_mode != "borderless");
    let _ = w.set_size(tauri::LogicalSize::new(gw, gh));
    match pos {
        Some((x, y)) => {
            let _ = w.set_position(tauri::LogicalPosition::new(x, y));
        }
        None => {
            let _ = w.center();
        }
    }
    if s.window_mode == "fullscreen" {
        let _ = w.set_fullscreen(true);
    }
    let _ = w.show();
    let _ = w.unminimize();
    let _ = w.set_focus();
    let st = app.state::<AppState>();
    st.game_shown.store(true, Ordering::Relaxed);
    *st.game_opened_ms.lock().unwrap() = presence::now_ms();
    if s.discord {
        let last = st.last_presence.lock().unwrap().clone().unwrap_or(("title".into(), String::new(), String::new()));
        set_presence(app, &last.0, &last.1, &last.2);
    }
    if let Some(l) = app.get_webview_window("launcher") {
        match s.on_play.as_str() {
            "keep" => {}
            "minimize" => {
                let _ = l.minimize();
            }
            _ => {
                let _ = l.hide();
            }
        }
    }
    logln!("play: {} window, {}x{}", s.window_mode, gw as u32, gh as u32);
}

fn open_game(app: &AppHandle, visible: bool) -> Result<(), String> {
    let app = app.clone();
    let st = app.state::<AppState>();
    if let Some(w) = app.get_webview_window("game") {
        if visible {
            show_game(&app, &w);
        }
        return Ok(());
    }
    if st.syncing.lock().unwrap().is_some() {
        return Err("wait for the download to finish".into());
    }
    reload_served(&app);
    if app.state::<serve::ServedState>().0.read().unwrap().is_none() {
        return Err("the game is not installed yet".into());
    }
    let mut s = store::load_settings(&app);
    let (gw, gh, _) = game_geometry(&app, &s);
    // webview2 serves custom schemes as http://<scheme>.localhost
    let url = if cfg!(windows) { "http://drsim.localhost/" } else { "drsim://localhost/" };
    let url = WebviewUrl::CustomProtocol(url.parse().map_err(|_| "bad url")?);
    let handle = app.clone();
    let mut b = WebviewWindowBuilder::new(&app, "game", url)
        .title("DELTARUNE Fight Simulator")
        .inner_size(gw, gh)
        .min_inner_size(320.0, 240.0)
        .center()
        .visible(false)
        .background_color(tauri::window::Color(0, 0, 0, 255))
        .initialization_script(game_init_script(&app, &s))
        .on_navigation(move |u| {
            let ours = u.scheme() == "drsim" || u.host_str() == Some("drsim.localhost") || u.scheme() == "blob" || u.scheme() == "data";
            if !ours {
                logln!("game: navigation to {} kept out of the game window", u.as_str().chars().take(120).collect::<String>());
                if u.scheme() == "https" || u.scheme() == "http" {
                    let _ = handle.opener().open_url(u.as_str(), None::<&str>);
                }
            }
            ours
        })
        .on_new_window({
            let handle = app.clone();
            move |u, _| {
                if u.scheme() == "https" || u.scheme() == "http" {
                    let _ = handle.opener().open_url(u.as_str(), None::<&str>);
                }
                tauri::webview::NewWindowResponse::Deny
            }
        });
    if let Some(d) = webview_data_dir(&app) {
        b = b.data_directory(d);
    }
    #[cfg(windows)]
    {
        b = b.additional_browser_args(perf::BROWSER_ARGS);
    }
    let w = b.build().map_err(|e| {
        logln!("play: the game window did not open: {e}");
        e.to_string()
    })?;
    perf::tune_webview(&w);
    if s.volume_dirty {
        s.volume_dirty = false;
        let _ = store::save_settings(&app, &s);
    }
    st.game_shown.store(false, Ordering::Relaxed);
    *st.last_presence.lock().unwrap() = None;
    if visible {
        show_game(&app, &w);
    } else {
        logln!("game: pre-warming (hidden)");
    }
    let a2 = app.clone();
    w.on_window_event(move |e| match e {
        tauri::WindowEvent::Destroyed => {
            logln!("game: closed");
            let st = a2.state::<AppState>();
            let was_shown = st.game_shown.swap(false, Ordering::Relaxed);
            a2.state::<presence::Presence>().clear();
            if st.fighting.swap(false, Ordering::Relaxed) {
                perf::fight_priority(false);
            }
            if !was_shown {
                return;
            }
            if st.quit_after_game.load(Ordering::Relaxed) {
                a2.exit(0);
                return;
            }
            show_launcher(&a2);
            let _ = a2.emit_to("launcher", "game-closed", ());
        }
        _ => {}
    });
    Ok(())
}

async fn drop_prewarm_wait(app: &AppHandle) {
    drop_prewarm(app);
    for _ in 0..40 {
        if app.get_webview_window("game").is_none() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn drop_prewarm(app: &AppHandle) {
    if !app.state::<AppState>().game_shown.load(Ordering::Relaxed) {
        if let Some(w) = app.get_webview_window("game") {
            let _ = w.destroy();
        }
    }
}

fn set_presence(app: &AppHandle, kind: &str, name: &str, chapter: &str) {
    let opened = *app.state::<AppState>().game_opened_ms.lock().unwrap();
    let (details, state, start) = match kind {
        "fight" if !name.is_empty() => (format!("Fighting {name}"), chapter.to_string(), presence::now_ms()),
        "minigame" => ("In a minigame".to_string(), name.to_string(), presence::now_ms()),
        "title" => ("On the title screen".to_string(), String::new(), opened),
        _ => ("In the menu".to_string(), String::new(), opened),
    };
    app.state::<presence::Presence>().set(presence::Status { details, state, start_ms: start });
}

fn webview_data_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    store::is_portable().then(|| store::base_dir(app).join("webview"))
}

#[tauri::command]
fn presence(app: AppHandle, st: State<'_, AppState>, kind: String, name: String, chapter: String) {
    let fight = kind == "fight" && st.game_shown.load(Ordering::Relaxed);
    if st.fighting.swap(fight, Ordering::Relaxed) != fight {
        perf::fight_priority(fight);
    }
    let s = store::load_settings(&app);
    if !s.discord {
        return;
    }
    let clean = |t: &str| t.chars().filter(|c| !c.is_control()).take(100).collect::<String>();
    let (kind, name, chapter) = (clean(&kind), clean(&name), clean(&chapter));
    *st.last_presence.lock().unwrap() = Some((kind.clone(), name.clone(), chapter.clone()));
    if st.game_shown.load(Ordering::Relaxed) {
        set_presence(&app, &kind, &name, &chapter);
    }
}

#[tauri::command]
async fn game_fullscreen(app: AppHandle) {
    if let Some(w) = app.get_webview_window("game") {
        let on = w.is_fullscreen().unwrap_or(false);
        let _ = w.set_fullscreen(!on);
    }
}

#[derive(Serialize)]
struct Notes {
    data: Option<serde_json::Value>,
    sprites: std::collections::BTreeMap<String, String>,
}

#[tauri::command]
async fn notes(app: AppHandle, online: bool) -> Result<Notes, String> {
    use base64::Engine;
    let s = store::load_settings(&app);
    let files = store::game_dir(&app, &s).join("files");
    let c = sync::client();
    let origins = store::origins();
    let online = online && !s.offline;
    let get = |rel: String| {
        let (c, origins, files) = (c.clone(), origins.clone(), files.clone());
        async move {
            if online {
                for o in &origins {
                    if let Ok(r) = c.get(format!("{o}/{rel}")).timeout(std::time::Duration::from_secs(6)).send().await {
                        if r.status().is_success() {
                            if let Ok(b) = r.bytes().await {
                                return Some(b.to_vec());
                            }
                        }
                    }
                }
            }
            std::fs::read(store::under(&files, &rel)).ok()
        }
    };
    let data: Option<serde_json::Value> = get("assets/updates/updates.json".into()).await.and_then(|b| serde_json::from_slice(&b).ok());
    let mut sprites = std::collections::BTreeMap::new();
    if let Some(list) = data.as_ref().and_then(|d| d.get("updates")).and_then(|u| u.as_array()) {
        let mut want = Vec::new();
        for e in list.iter().take(12) {
            for sp in e.get("sprite").and_then(|s| s.as_array()).into_iter().flatten() {
                if let (Some(n), Some(f)) = (sp.get(0).and_then(|v| v.as_str()), sp.get(1).and_then(|v| v.as_u64())) {
                    let key = format!("{n}_{f}");
                    if store::safe_rel(&key) && !want.contains(&key) {
                        want.push(key);
                    }
                }
            }
        }
        let got = futures_util::future::join_all(want.iter().map(|k| get(format!("assets/updates/spr/{k}.png")))).await;
        for (k, b) in want.into_iter().zip(got) {
            if let Some(b) = b.filter(|b| b.starts_with(b"\x89PNG")) {
                sprites.insert(k, format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)));
            }
        }
    }
    Ok(Notes { data, sprites })
}

#[tauri::command]
fn set_setting(app: AppHandle, key: String, value: serde_json::Value) -> Result<store::Settings, String> {
    let mut s = store::load_settings(&app);
    let b = || value.as_bool().ok_or("expected on/off");
    let n = || value.as_u64().ok_or("expected a number");
    let t = |allowed: &[&str]| value.as_str().filter(|v| allowed.contains(v)).map(|v| v.to_string()).ok_or("not an option");
    match key.as_str() {
        "window_mode" => s.window_mode = t(&["windowed", "borderless", "fullscreen"])?,
        "window_scale" => s.window_scale = n()?.min(4) as u32,
        "start_in_game" => s.start_in_game = b()?,
        "on_play" => s.on_play = t(&["hide", "minimize", "keep"])?,
        "close_to_tray" => s.close_to_tray = b()?,
        "channel" => s.channel = t(&["stable", "beta"])?,
        "auto_check" => s.auto_check = b()?,
        "offline" => s.offline = b()?,
        "perf" => s.perf = b()?,
        "music" => {
            s.music = n()?.min(100) as u32;
            s.volume_dirty = true;
        }
        "sfx" => {
            s.sfx = n()?.min(100) as u32;
            s.volume_dirty = true;
        }
        "discord" => {
            s.discord = b()?;
            if !s.discord {
                app.state::<presence::Presence>().clear();
            }
        }
        "autostart" => {
            let on = b()?;
            let al = app.autolaunch();
            let r = if on { al.enable() } else { al.disable() };
            r.map_err(|e| {
                logln!("autostart: {e}");
                format!("could not change the startup setting: {e}")
            })?;
            s.autostart = on;
        }
        _ => return Err("unknown setting".into()),
    }
    logln!("setting {key} = {value}");
    if matches!(key.as_str(), "music" | "sfx" | "perf" | "channel" | "offline") {
        let a = app.clone();
        tauri::async_runtime::spawn(async move { drop_prewarm(&a) });
    }
    store::save_settings(&app, &s)?;
    Ok(s)
}

#[tauri::command]
async fn pick_install_dir(app: AppHandle, st: State<'_, AppState>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    drop_prewarm_wait(&app).await;
    if st.syncing.lock().unwrap().is_some() || app.get_webview_window("game").is_some() {
        return Err("wait for the download to finish and close the game first".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut d = app.dialog().file().set_title("Install location for the game data");
    if let Some(w) = app.get_webview_window("launcher") {
        d = d.set_parent(&w);
    }
    d.pick_folder(move |p| {
        let _ = tx.send(p);
    });
    let Some(picked) = rx.await.ok().flatten() else { return Ok(None) };
    let picked = picked.into_path().map_err(|e| e.to_string())?;
    let mut s = store::load_settings(&app);
    let old = store::game_dir(&app, &s);
    let new = picked.join("DELTARUNE Fight Simulator");
    if new == old {
        return Ok(Some(new.display().to_string()));
    }
    if new.starts_with(&old) {
        return Err("pick a folder outside the current install folder".into());
    }
    if old.join("state.json").is_file() {
        let (o, n) = (old.clone(), new.clone());
        tauri::async_runtime::spawn_blocking(move || move_dir(&o, &n)).await.map_err(|e| e.to_string())??;
    } else {
        std::fs::create_dir_all(&new).map_err(|e| format!("cannot write there: {e}"))?;
    }
    logln!("install location: {} -> {}", old.display(), new.display());
    s.install_dir = Some(new.display().to_string());
    store::save_settings(&app, &s)?;
    reload_served(&app);
    Ok(Some(new.display().to_string()))
}

fn move_dir(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    if to.exists() && std::fs::read_dir(to).map(|mut d| d.next().is_some()).unwrap_or(false) {
        return Err("that folder already has a \"DELTARUNE Fight Simulator\" folder in it".into());
    }
    if let Some(p) = to.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir(to);
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fn copy(a: &std::path::Path, b: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(b)?;
        for e in std::fs::read_dir(a)? {
            let e = e?;
            let t = b.join(e.file_name());
            if e.file_type()?.is_dir() {
                copy(&e.path(), &t)?;
            } else {
                std::fs::copy(e.path(), t)?;
            }
        }
        Ok(())
    }
    copy(from, to).map_err(|e| {
        let _ = std::fs::remove_dir_all(to);
        format!("could not move the game data: {e}")
    })?;
    let _ = std::fs::remove_dir_all(from);
    Ok(())
}

fn dir_size(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p)
        .map(|d| d.flatten().map(|e| if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { dir_size(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) }).sum())
        .unwrap_or(0)
}

#[tauri::command]
fn clear_cache(app: AppHandle, st: State<'_, AppState>) -> Result<u64, String> {
    if st.syncing.lock().unwrap().is_some() {
        return Err("a download is running".into());
    }
    let mut s = store::load_settings(&app);
    let staging = store::game_dir(&app, &s).join("staging");
    let freed = dir_size(&staging);
    let _ = std::fs::remove_dir_all(&staging);
    s.clear_cache_next = true;
    store::save_settings(&app, &s)?;
    logln!("clear cache: {freed} bytes of unfinished downloads; WebView caches at the next start");
    Ok(freed)
}

fn clear_webview_caches(app: &AppHandle) {
    let root = webview_data_dir(app).or_else(|| app.path().app_local_data_dir().ok());
    if let Some(root) = root {
        for sub in ["Cache", "Code Cache", "GPUCache", "DawnGraphiteCache", "DawnWebGPUCache"] {
            let p = root.join("EBWebView").join("Default").join(sub);
            if p.exists() {
                let _ = std::fs::remove_dir_all(&p);
            }
        }
    }
}

#[tauri::command]
fn open_link(app: AppHandle, which: String) -> Result<(), String> {
    let s = store::load_settings(&app);
    let target = match which.as_str() {
        "site" => format!("{}/", store::SITE),
        "discord" => DISCORD_INVITE.to_string(),
        "updates" => format!("{}/updates", store::SITE),
        "portable" => format!("{}/desktop/DRFightSim-portable.exe", store::SITE),
        "logs" => return app.opener().open_path(logs::dir().ok_or("no log folder")?.display().to_string(), None::<&str>).map_err(|e| e.to_string()),
        "data" => {
            let d = store::game_dir(&app, &s);
            let _ = std::fs::create_dir_all(&d);
            return app.opener().open_path(d.display().to_string(), None::<&str>).map_err(|e| e.to_string());
        }
        _ => return Err("unknown link".into()),
    };
    app.opener().open_url(target, None::<&str>).map_err(|e| e.to_string())
}

fn shortcut_paths(app: &AppHandle) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    if store::is_portable() {
        return None;
    }
    let name = format!("{}.lnk", app.package_info().name);
    Some((app.path().desktop_dir().ok()?.join(&name), store::base_dir(app).join(name)))
}

#[tauri::command]
fn desktop_shortcut(app: AppHandle, on: bool) -> Result<bool, String> {
    let (lnk, kept) = shortcut_paths(&app).ok_or("no shortcut")?;
    let (from, to) = if on { (kept, lnk.clone()) } else { (lnk.clone(), kept) };
    if from.is_file() {
        if let Some(d) = to.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::rename(&from, &to).or_else(|_| std::fs::copy(&from, &to).and_then(|_| std::fs::remove_file(&from))).map_err(|e| e.to_string())?;
        logln!("desktop shortcut: {}", if on { "on" } else { "off" });
    }
    Ok(lnk.is_file())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    logln!("quit");
    app.exit(0);
}

#[tauri::command]
async fn install_launcher_update(app: AppHandle, st: State<'_, AppState>) -> Result<(), String> {
    if store::is_portable() {
        return Err("portable".into());
    }
    let up = st.launcher_update.lock().unwrap().take().ok_or("no launcher update")?;
    logln!("launcher update: installing v{}", up.version);
    let a = app.clone();
    let mut got = 0u64;
    up.download_and_install(
        move |n, total| {
            got += n as u64;
            let _ = a.emit_to("launcher", "launcher-progress", (got, total.unwrap_or(0)));
        },
        || {},
    )
    .await
    .map_err(|e| {
        logln!("launcher update failed: {e}");
        e.to_string()
    })?;
    app.restart();
}

fn tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open launcher", true, None::<&str>)?;
    let play_i = MenuItem::with_id(app, "play", "Play", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &play_i, &PredefinedMenuItem::separator(app)?, &quit])?;
    let mut t = TrayIconBuilder::with_id("main").tooltip("DELTARUNE Fight Simulator").menu(&menu).show_menu_on_left_click(false);
    if let Some(i) = app.default_window_icon() {
        t = t.icon(i.clone());
    }
    t.on_menu_event(|app, e| match e.id().as_ref() {
        "open" => show_launcher(app),
        "play" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if open_game(&app, true).is_err() {
                    show_launcher(&app);
                }
            });
        }
        "quit" => app.exit(0),
        _ => {}
    })
    .on_tray_icon_event(|t, e| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            let app = t.app_handle();
            match app.get_webview_window("game").filter(|_| app.state::<AppState>().game_shown.load(Ordering::Relaxed)) {
                Some(g) => {
                    let _ = g.unminimize();
                    let _ = g.set_focus();
                }
                None => show_launcher(app),
            }
        }
    })
    .build(app)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| match app.get_webview_window("game").filter(|_| app.state::<AppState>().game_shown.load(Ordering::Relaxed)) {
            Some(g) => {
                let _ = g.unminimize();
                let _ = g.set_focus();
            }
            None => show_launcher(app),
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--autostart"])))
        .manage(AppState::default())
        .manage(serve::ServedState::default())
        .manage(presence::Presence::start())
        .register_asynchronous_uri_scheme_protocol("drsim", |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            if serve::is_api(&req) {
                if store::load_settings(&app).offline {
                    responder.respond(serve::offline());
                    return;
                }
                tauri::async_runtime::spawn(async move { responder.respond(serve::forward(sync::client(), req).await) });
                return;
            }
            let sv = app.state::<serve::ServedState>().0.read().unwrap().clone();
            tauri::async_runtime::spawn_blocking(move || responder.respond(serve::handle(sv, &req)));
        })
        .setup(|app| {
            let h = app.handle().clone();
            logs::init(&store::base_dir(&h).join("logs"));
            let autostarted = std::env::args().any(|a| a == "--autostart");
            let mut s = store::load_settings(&h);
            logln!(
                "start: launcher v{} ({}{}{}), webview {}, data {}",
                h.package_info().version,
                std::env::consts::OS,
                if store::is_portable() { ", portable" } else { "" },
                if std::env::args().any(|a| a == "--first-run") { ", first run" } else { "" },
                tauri::webview_version().unwrap_or_default(),
                store::base_dir(&h).display()
            );
            if s.clear_cache_next {
                clear_webview_caches(&h);
                s.clear_cache_next = false;
                let _ = store::save_settings(&h, &s);
                logln!("clear cache: WebView caches removed");
            }
            h.state::<AppState>().started_hidden.store(autostarted, Ordering::Relaxed);
            reload_served(&h);
            let mut b = WebviewWindowBuilder::new(app, "launcher", WebviewUrl::App("index.html".into()))
                .title("DELTARUNE Fight Simulator")
                .inner_size(880.0, 520.0)
                .resizable(false)
                .maximizable(false)
                .decorations(false)
                .shadow(true)
                .center()
                .visible(false)
                .background_color(tauri::window::Color(0, 0, 0, 255));
            if let Some(d) = webview_data_dir(&h) {
                b = b.data_directory(d);
            }
            #[cfg(windows)]
            {
                b = b.additional_browser_args(perf::BROWSER_ARGS);
            }
            let w = b.build()?;
            perf::tune_webview(&w);
            let h2 = h.clone();
            w.on_window_event(move |e| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                    let s = store::load_settings(&h2);
                    let game_open = h2.state::<AppState>().game_shown.load(Ordering::Relaxed);
                    if s.close_to_tray || game_open {
                        api.prevent_close();
                        if let Some(l) = h2.get_webview_window("launcher") {
                            let _ = l.hide();
                        }
                        if game_open && !s.close_to_tray {
                            h2.state::<AppState>().quit_after_game.store(true, Ordering::Relaxed);
                        }
                    } else {
                        logln!("quit");
                        h2.exit(0);
                    }
                }
            });
            if let Err(e) = tray(&h) {
                logln!("tray: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            launcher_state,
            check,
            sync,
            verify,
            cancel_sync,
            play,
            prewarm,
            presence,
            game_fullscreen,
            notes,
            set_setting,
            pick_install_dir,
            clear_cache,
            open_link,
            desktop_shortcut,
            quit_app,
            install_launcher_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running the launcher");
}
