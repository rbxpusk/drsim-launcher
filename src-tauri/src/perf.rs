
// tauri's defaults first, it drops them once custom args are set
#[cfg(windows)]
pub const BROWSER_ARGS: &str = "--autoplay-policy=no-user-gesture-required --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --force_high_performance_gpu --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";

pub fn tune_webview(w: &tauri::WebviewWindow) {
    #[cfg(target_os = "linux")]
    {
        let _ = w.with_webview(|wv| {
            use webkit2gtk::{HardwareAccelerationPolicy, SettingsExt, WebViewExt};
            if let Some(s) = WebViewExt::settings(&wv.inner()) {
                s.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Always);
                s.set_enable_webgl(true);
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = w;
}

pub fn fight_priority(on: bool) {
    #[cfg(windows)]
    std::thread::spawn(move || {
        let n = win::set_tree_priority(on);
        crate::logln!("priority: {} for {n} processes", if on { "above normal" } else { "normal" });
    });
    #[cfg(not(windows))]
    let _ = on;
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_SET_INFORMATION};

    fn processes() -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snap, &mut e) != 0 {
                loop {
                    out.push((e.th32ProcessID, e.th32ParentProcessID));
                    if Process32NextW(snap, &mut e) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snap);
        }
        out
    }

    pub fn set_tree_priority(on: bool) -> usize {
        let me = unsafe { GetCurrentProcessId() };
        let all = processes();
        let mut tree = vec![me];
        let mut i = 0;
        while i < tree.len() {
            let p = tree[i];
            for &(pid, ppid) in &all {
                if ppid == p && pid != p && !tree.contains(&pid) {
                    tree.push(pid);
                }
            }
            i += 1;
        }
        let class = if on { ABOVE_NORMAL_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS };
        let mut n = 0;
        for pid in tree {
            unsafe {
                let h = OpenProcess(PROCESS_SET_INFORMATION, 0, pid);
                if !h.is_null() {
                    if SetPriorityClass(h, class) != 0 {
                        n += 1;
                    }
                    CloseHandle(h);
                }
            }
        }
        n
    }
}
