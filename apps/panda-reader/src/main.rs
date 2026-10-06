#![recursion_limit = "512"]
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod request_epoch;
mod services;
mod ui;
#[allow(dead_code)]
mod updater;

fn main() {
    let data_dir = updater::data_dir();
    if updater::apply_pending_on_launch(&data_dir) {
        return;
    }
    app::run();
}
