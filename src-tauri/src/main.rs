#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// ask hybrid-gpu laptops for the fast gpu
#[cfg(windows)]
#[no_mangle]
#[used]
pub static NvOptimusEnablement: u32 = 1;
#[cfg(windows)]
#[no_mangle]
#[used]
pub static AmdPowerXpressRequestHighPerformance: i32 = 1;

fn main() {
    drsim_launcher_lib::run();
}
