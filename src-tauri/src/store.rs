use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub const SITE: &str = "https://deltarunesim.com";
pub const PAGES: &str = "https://deltarunesim.pages.dev";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub v: u32,
    pub version: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub built: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub files: BTreeMap<String, FileEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub install_dir: Option<String>,
    pub window_mode: String,
    pub window_scale: u32,
    pub start_in_game: bool,
    pub on_play: String,
    pub close_to_tray: bool,
    pub autostart: bool,
    pub channel: String,
    pub auto_check: bool,
    pub offline: bool,
    pub perf: bool,
    pub music: u32,
    pub sfx: u32,
    pub volume_dirty: bool,
    pub discord: bool,
    pub clear_cache_next: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Settings {
            install_dir: None,
            window_mode: "windowed".into(),
            window_scale: 0,
            start_in_game: false,
            on_play: "hide".into(),
            close_to_tray: false,
            autostart: false,
            channel: "stable".into(),
            auto_check: true,
            offline: false,
            perf: false,
            music: 60,
            sfx: 100,
            volume_dirty: false,
            discord: true,
            clear_cache_next: false,
        }
    }
}

pub fn is_portable() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let stem = exe.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    stem.ends_with("portable") || exe.parent().map(|d| d.join("portable").is_file()).unwrap_or(false)
}

pub fn base_dir(app: &AppHandle) -> PathBuf {
    if is_portable() {
        if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("DRSimData"))) {
            return d;
        }
    }
    app.path().app_local_data_dir().unwrap_or_else(|_| PathBuf::from("DRSimData"))
}

pub fn load_settings(app: &AppHandle) -> Settings {
    std::fs::read(base_dir(app).join("settings.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_settings(app: &AppHandle, s: &Settings) -> Result<(), String> {
    let p = base_dir(app).join("settings.json");
    write_atomic(&p, &serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?)
}

pub fn game_dir(app: &AppHandle, s: &Settings) -> PathBuf {
    match &s.install_dir {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => base_dir(app).join("game"),
    }
}

pub fn load_manifest(p: &Path) -> Option<Manifest> {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice::<Manifest>(&b).ok())
}

pub fn write_atomic(p: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
}

pub fn safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && rel.len() <= 255
        && rel.bytes().all(|b| b.is_ascii_alphanumeric() || b"_@./ -".contains(&b))
        && !rel.starts_with('/')
        && rel.split('/').all(|s| !s.is_empty() && !s.starts_with('.') && !s.ends_with('.') && !s.ends_with(' '))
}

pub fn under(dir: &Path, rel: &str) -> PathBuf {
    let mut p = dir.to_path_buf();
    for s in rel.split('/') {
        p.push(s);
    }
    p
}

pub fn origins() -> Vec<String> {
    // local test server override
    if let Ok(o) = std::env::var("DRSIM_ORIGIN") {
        let o = o.trim_end_matches('/').to_string();
        if o.starts_with("http://127.0.0.1:") || o.starts_with("http://localhost:") {
            return vec![o];
        }
    }
    vec![SITE.to_string(), PAGES.to_string()]
}

pub fn free_space(p: &Path) -> Option<u64> {
    let mut q = p.to_path_buf();
    while !q.exists() {
        q = q.parent()?.to_path_buf();
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let w: Vec<u16> = q.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut free = 0u64;
        let ok = unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(w.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
        if ok != 0 {
            Some(free)
        } else {
            None
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(q.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0 {
            Some(st.f_bavail as u64 * st.f_frsize as u64)
        } else {
            None
        }
    }
}
