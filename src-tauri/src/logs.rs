use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static LOG: OnceLock<Mutex<PathBuf>> = OnceLock::new();

pub fn init(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let _ = LOG.set(Mutex::new(dir.join("launcher.log")));
}

pub fn dir() -> Option<PathBuf> {
    LOG.get().and_then(|m| m.lock().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())))
}

pub fn stamp() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (s.div_euclid(86400), s.rem_euclid(86400));
    // days since epoch to y-m-d (howard hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub fn line(msg: impl AsRef<str>) {
    let Some(m) = LOG.get() else { return };
    let Ok(p) = m.lock() else { return };
    if std::fs::metadata(&*p).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&*p, p.with_file_name("launcher.old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&*p) {
        let _ = writeln!(f, "{} {}", stamp(), msg.as_ref());
    }
}

#[macro_export]
macro_rules! logln {
    ($($t:tt)*) => { $crate::logs::line(format!($($t)*)) };
}
