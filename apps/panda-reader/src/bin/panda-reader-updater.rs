#[path = "../updater.rs"]
#[allow(dead_code)]
mod updater;

fn main() {
    if let Err(error) = updater::helper_main() {
        eprintln!("Panda Reader update failed: {error}");
        std::process::exit(1);
    }
}
