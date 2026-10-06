//! Release discovery, verified staging, and platform update hand-off.

use directories::ProjectDirs;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

const RELEASES_API: &str = "https://api.github.com/repos/yuhangch/panda-reader/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/yuhangch/panda-reader/releases";
const MAX_ASSET_BYTES: u64 = 1_500_000_000;
static CHECK_RUNNING: AtomicBool = AtomicBool::new(false);

pub fn data_dir() -> PathBuf {
    ProjectDirs::from("com", "PandaReader", "PandaReader")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("data"))
}

#[derive(Clone, Debug)]
pub enum UpdateStatus {
    Idle,
    Checking,
    Downloading {
        version: String,
        received: u64,
        total: u64,
    },
    UpToDate,
    Ready(ReadyUpdate),
    NextLaunch(ReadyUpdate),
    Later(ReadyUpdate),
    ManualRequired {
        version: String,
        reason: String,
        release_url: String,
    },
    Failed(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadyUpdate {
    pub version: String,
    pub package: PathBuf,
    pub sha256: String,
    pub release_url: String,
    plan: InstallPlan,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InstallPlan {
    kind: InstallKind,
    target: PathBuf,
    helper: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum InstallKind {
    WindowsSetup,
    MacAppZip,
    LinuxAppImage,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

pub fn start_check(data_dir: PathBuf, events: UnboundedSender<UpdateStatus>) {
    if CHECK_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    if std::thread::Builder::new()
        .name("panda-reader-updater".into())
        .spawn(move || {
            let _ = events.send(UpdateStatus::Checking);
            let status =
                check_and_download(&data_dir, &events).unwrap_or_else(UpdateStatus::Failed);
            let _ = events.send(status);
            CHECK_RUNNING.store(false, Ordering::Release);
        })
        .is_err()
    {
        CHECK_RUNNING.store(false, Ordering::Release);
    }
}

fn check_and_download(
    data_dir: &Path,
    events: &UnboundedSender<UpdateStatus>,
) -> Result<UpdateStatus, String> {
    let client = Client::builder()
        .user_agent(concat!("PandaReader/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let release = client
        .get(RELEASES_API)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?
        .json::<GithubRelease>()
        .map_err(|error| error.to_string())?;
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let latest = semver::Version::parse(version).map_err(|_| {
        format!(
            "The latest release has an invalid version: {}",
            release.tag_name
        )
    })?;
    let current =
        semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|error| error.to_string())?;
    if latest <= current {
        return Ok(UpdateStatus::UpToDate);
    }

    let release_url = if release
        .html_url
        .starts_with("https://github.com/yuhangch/panda-reader/")
    {
        release.html_url
    } else {
        RELEASES_PAGE.to_owned()
    };
    let plan = match detect_installation() {
        Ok(plan) => plan,
        Err(reason) => {
            return Ok(UpdateStatus::ManualRequired {
                version: latest.to_string(),
                reason,
                release_url,
            });
        }
    };
    let asset_name = package_name(&latest.to_string(), plan.kind);
    let Some(asset) = release.assets.iter().find(|asset| asset.name == asset_name) else {
        return Ok(UpdateStatus::ManualRequired {
            version: latest.to_string(),
            reason: format!("This release does not include {asset_name} for this installation."),
            release_url,
        });
    };
    if asset.size == 0 || asset.size > MAX_ASSET_BYTES {
        return Err(format!(
            "The update package size is invalid: {} bytes",
            asset.size
        ));
    }
    let Some(checksums) = release
        .assets
        .iter()
        .find(|asset| asset.name == "checksums.txt")
    else {
        return Ok(UpdateStatus::ManualRequired {
            version: latest.to_string(),
            reason: "This release has no checksums.txt, so automatic installation is disabled."
                .into(),
            release_url,
        });
    };
    let checksum_text = client
        .get(&checksums.browser_download_url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?
        .text()
        .map_err(|error| error.to_string())?;
    let expected_sha256 = checksum_for(&checksum_text, &asset.name)
        .ok_or_else(|| format!("checksums.txt has no entry for {}", asset.name))?;

    let cache_dir = data_dir.join("updates");
    fs::create_dir_all(&cache_dir).map_err(|error| error.to_string())?;
    let package = cache_dir.join(&asset.name);
    if !package.is_file() || hash_file(&package)? != expected_sha256 {
        let partial = cache_dir.join(format!("{}.part", asset.name));
        let _ = fs::remove_file(&partial);
        let mut response = client
            .get(&asset.browser_download_url)
            .timeout(Duration::from_secs(300))
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|error| error.to_string())?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_ASSET_BYTES)
        {
            return Err("The update package exceeds the 1.5 GB safety limit.".into());
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        let mut received = 0_u64;
        let mut buffer = [0_u8; 256 * 1024];
        let _ = events.send(UpdateStatus::Downloading {
            version: latest.to_string(),
            received,
            total: asset.size,
        });
        loop {
            let read = response
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            received = received
                .checked_add(read as u64)
                .ok_or_else(|| "The update package size overflowed.".to_owned())?;
            if received > MAX_ASSET_BYTES || received > asset.size {
                return Err("The downloaded package exceeded its declared size.".into());
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
            hasher.update(&buffer[..read]);
            let _ = events.send(UpdateStatus::Downloading {
                version: latest.to_string(),
                received,
                total: asset.size,
            });
        }
        output.sync_all().map_err(|error| error.to_string())?;
        drop(output);
        if received != asset.size {
            let _ = fs::remove_file(&partial);
            return Err(format!(
                "The downloaded package is incomplete ({received} of {} bytes).",
                asset.size
            ));
        }
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected_sha256 {
            let _ = fs::remove_file(&partial);
            return Err("The update package failed SHA-256 verification.".into());
        }
        let _ = fs::remove_file(&package);
        fs::rename(&partial, &package).map_err(|error| error.to_string())?;
    }

    Ok(UpdateStatus::Ready(ReadyUpdate {
        version: latest.to_string(),
        package,
        sha256: expected_sha256,
        release_url,
        plan,
    }))
}

fn checksum_for(contents: &str, file_name: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let mut columns = line.split_whitespace();
        let hash = columns.next()?;
        let name = columns.next()?.trim_start_matches('*');
        (name == file_name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn package_name(version: &str, kind: InstallKind) -> String {
    match kind {
        InstallKind::WindowsSetup => format!("panda-reader-{version}-windows-x86_64-setup.exe"),
        InstallKind::MacAppZip => format!(
            "panda-reader-{version}-macos-{}.zip",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x86_64"
            }
        ),
        InstallKind::LinuxAppImage => format!("panda-reader-{version}-linux-x86_64.AppImage"),
    }
}

fn detect_installation() -> Result<InstallPlan, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    {
        let local_app_data = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| {
                "Could not locate this user's Windows application directory.".to_owned()
            })?;
        let expected_root = local_app_data.join("Programs").join("Panda Reader");
        let root = executable
            .parent()
            .ok_or_else(|| "Could not locate the Panda Reader installation folder.".to_owned())?;
        let normalized_root = root.to_string_lossy().to_lowercase();
        let normalized_expected = expected_root.to_string_lossy().to_lowercase();
        if !normalized_root.starts_with(&normalized_expected)
            || !root.join("unins000.exe").is_file()
        {
            return Err("Automatic updates support the per-user Windows installer. This copy appears to be a portable or all-users installation.".into());
        }
        let helper = root.join("panda-reader-updater.exe");
        if !helper.is_file() {
            return Err("This installation predates the in-app updater. Install one update manually to enable future in-app updates.".into());
        }
        return Ok(InstallPlan {
            kind: InstallKind::WindowsSetup,
            target: executable,
            helper,
        });
    }
    #[cfg(target_os = "macos")]
    {
        let bundle = executable
            .ancestors()
            .find(|path| path.extension().is_some_and(|extension| extension == "app"))
            .ok_or_else(|| "Panda Reader is not running from an application bundle.".to_owned())?;
        let parent = bundle
            .parent()
            .ok_or_else(|| "Could not locate the application folder.".to_owned())?;
        if !writable_directory(parent) {
            return Err("This application is in a folder that cannot be updated by this user. Move it to a writable Applications folder or update it manually.".into());
        }
        let helper = bundle.join("Contents/MacOS/panda-reader-updater");
        if !helper.is_file() {
            return Err("This installation predates the in-app updater. Install one update manually to enable future in-app updates.".into());
        }
        return Ok(InstallPlan {
            kind: InstallKind::MacAppZip,
            target: bundle.to_path_buf(),
            helper,
        });
    }
    #[cfg(target_os = "linux")]
    {
        let app_image = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .ok_or_else(|| "In-app updates currently support the AppImage installation. Tarball installs use the release page.".to_owned())?;
        let parent = app_image
            .parent()
            .ok_or_else(|| "Could not locate the AppImage folder.".to_owned())?;
        if !writable_directory(parent) {
            return Err("The AppImage is in a folder that cannot be updated by this user.".into());
        }
        let helper = executable
            .parent()
            .ok_or_else(|| "Could not locate the AppImage helper.".to_owned())?
            .join("panda-reader-updater");
        if !helper.is_file() {
            return Err("This installation predates the in-app updater. Install one update manually to enable future in-app updates.".into());
        }
        return Ok(InstallPlan {
            kind: InstallKind::LinuxAppImage,
            target: app_image,
            helper,
        });
    }
    #[allow(unreachable_code)]
    Err("Automatic updates are not supported for this platform.".into())
}

#[cfg(unix)]
fn writable_directory(path: &Path) -> bool {
    let probe = path.join(format!(".panda-reader-update-check-{}", std::process::id()));
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

pub fn save_for_next_launch(data_dir: &Path, update: &ReadyUpdate) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
    let pending = data_dir.join("pending-update.json");
    let temporary = data_dir.join("pending-update.json.part");
    let bytes = serde_json::to_vec_pretty(update).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    if pending.exists() {
        fs::remove_file(&pending).map_err(|error| error.to_string())?;
    }
    fs::rename(temporary, pending).map_err(|error| error.to_string())
}

pub fn launch_now(data_dir: &Path, update: &ReadyUpdate) -> Result<(), String> {
    save_for_next_launch(data_dir, update)?;
    match spawn_helper(update, data_dir, std::process::id()) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(data_dir.join("pending-update.json"));
            Err(error)
        }
    }
}

fn spawn_helper(update: &ReadyUpdate, data_dir: &Path, parent_pid: u32) -> Result<(), String> {
    if hash_file(&update.package)? != update.sha256 {
        return Err("The staged update no longer matches its verified SHA-256 checksum.".into());
    }
    let helper_name = if cfg!(windows) {
        format!("panda-reader-updater-{}.exe", std::process::id())
    } else {
        format!("panda-reader-updater-{}", std::process::id())
    };
    let helper_copy = std::env::temp_dir().join(helper_name);
    fs::copy(&update.plan.helper, &helper_copy).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(&helper_copy)
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&helper_copy, permissions).map_err(|error| error.to_string())?;
    }
    let kind = match update.plan.kind {
        InstallKind::WindowsSetup => "windows-setup",
        InstallKind::MacAppZip => "mac-app-zip",
        InstallKind::LinuxAppImage => "linux-appimage",
    };
    let mut command = Command::new(helper_copy);
    command
        .arg("--apply-update")
        .arg(kind)
        .arg(parent_pid.to_string())
        .arg(&update.package)
        .arg(&update.plan.target)
        .arg(&update.version)
        .arg(&update.sha256)
        .arg(data_dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command.spawn().map_err(|error| error.to_string())?;
    Ok(())
}

/// If the user chose "Next launch", hand off before creating the GPUI app.
/// Returns true when the updater was started and the current process should exit.
pub fn apply_pending_on_launch(data_dir: &Path) -> bool {
    let pending_path = data_dir.join("pending-update.json");
    let Ok(bytes) = fs::read(&pending_path) else {
        return false;
    };
    let Ok(update) = serde_json::from_slice::<ReadyUpdate>(&bytes) else {
        let _ = fs::remove_file(pending_path);
        return false;
    };
    let Ok(current) = semver::Version::parse(env!("CARGO_PKG_VERSION")) else {
        return false;
    };
    let Ok(version) = semver::Version::parse(&update.version) else {
        let _ = fs::remove_file(pending_path);
        return false;
    };
    if version <= current {
        let _ = fs::remove_file(pending_path);
        return false;
    }
    let Ok(expected_plan) = detect_installation() else {
        return false;
    };
    if expected_plan.target != update.plan.target || expected_plan.kind != update.plan.kind {
        let _ = fs::remove_file(pending_path);
        return false;
    }
    match spawn_helper(&update, data_dir, std::process::id()) {
        Ok(()) => true,
        Err(error) => {
            let _ = fs::remove_file(pending_path);
            let _ = fs::write(data_dir.join("update-failure.txt"), error);
            false
        }
    }
}

pub fn helper_main() -> Result<(), String> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.len() != 8 || arguments[0] != "--apply-update" {
        return Err("Expected --apply-update and a verified update plan.".into());
    }
    let kind = arguments[1]
        .to_str()
        .ok_or_else(|| "Invalid update format.".to_owned())?;
    let parent_pid = arguments[2]
        .to_str()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| "Invalid parent process ID.".to_owned())?;
    let package = PathBuf::from(&arguments[3]);
    let target = PathBuf::from(&arguments[4]);
    let version = arguments[5]
        .to_str()
        .ok_or_else(|| "Invalid update version.".to_owned())?;
    let expected_sha256 = arguments[6]
        .to_str()
        .ok_or_else(|| "Invalid update checksum.".to_owned())?;
    let data_dir = PathBuf::from(&arguments[7]);
    let version = semver::Version::parse(version).map_err(|error| error.to_string())?;
    if hash_file(&package)? != expected_sha256 {
        return Err("The staged update failed SHA-256 verification.".into());
    }
    wait_for_parent(parent_pid)?;
    let result = match kind {
        "windows-setup" => apply_windows_setup(&package, &target, &version),
        "mac-app-zip" => apply_mac_bundle(&package, &target, &version),
        "linux-appimage" => apply_linux_appimage(&package, &target, &version),
        _ => Err("Unknown update format.".into()),
    };
    match result {
        Ok(()) => {
            let _ = fs::remove_file(data_dir.join("pending-update.json"));
            let _ = fs::remove_file(data_dir.join("update-failure.txt"));
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(data_dir.join("pending-update.json"));
            let _ = fs::write(data_dir.join("update-failure.txt"), &error);
            Err(error)
        }
    }
}

fn wait_for_parent(pid: u32) -> Result<(), String> {
    let until = std::time::Instant::now() + Duration::from_secs(180);
    while process_is_running(pid) {
        if std::time::Instant::now() >= until {
            return Err("Timed out waiting for Panda Reader to close.".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    // SAFETY: kill with signal 0 only checks process existence and does not send a signal.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: the handle is queried only for this process and always closed below.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0;
        let running = GetExitCodeProcess(handle, &mut code) != 0 && code == STILL_ACTIVE as u32;
        CloseHandle(handle);
        running
    }
}

#[cfg(target_os = "windows")]
fn apply_windows_setup(
    package: &Path,
    target: &Path,
    version: &semver::Version,
) -> Result<(), String> {
    let backup = std::env::temp_dir().join(format!("panda-reader-{}-old.exe", std::process::id()));
    fs::copy(target, &backup)
        .map_err(|error| format!("Could not back up the current app: {error}"))?;
    let status = Command::new(package)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"])
        .status()
        .map_err(|error| format!("Could not run the installer: {error}"))?;
    if !status.success() {
        let _ = fs::copy(&backup, target);
        return Err(format!("The installer exited with {status}."));
    }
    let launch = Command::new(target).spawn();
    if let Err(error) = launch {
        let _ = fs::copy(&backup, target);
        return Err(format!(
            "Could not relaunch Panda Reader {version}: {error}"
        ));
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

#[cfg(target_os = "macos")]
fn apply_mac_bundle(
    package: &Path,
    target: &Path,
    version: &semver::Version,
) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "Could not locate the application folder.".to_owned())?;
    let stage = parent.join(format!(".panda-reader-update-{}", std::process::id()));
    let extracted = stage.join("payload");
    fs::create_dir_all(&extracted).map_err(|error| error.to_string())?;
    let status = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(package)
        .arg(&extracted)
        .status()
        .map_err(|error| format!("Could not unpack the update: {error}"))?;
    if !status.success() {
        let _ = fs::remove_dir_all(&stage);
        return Err("Could not unpack the macOS update.".into());
    }
    let staged_bundle = extracted.join("Panda Reader.app");
    if !staged_bundle.join("Contents/MacOS/panda-reader").is_file() {
        let _ = fs::remove_dir_all(&stage);
        return Err("The downloaded package is not a Panda Reader application.".into());
    }
    let signature = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&staged_bundle)
        .status()
        .map_err(|error| format!("Could not verify the app signature: {error}"))?;
    if !signature.success() {
        let _ = fs::remove_dir_all(&stage);
        return Err("The downloaded app signature could not be verified.".into());
    }
    let backup = parent.join(format!(".Panda Reader.app.backup-{}", std::process::id()));
    fs::rename(target, &backup)
        .map_err(|error| format!("Could not back up the installed app: {error}"))?;
    if let Err(error) = fs::rename(&staged_bundle, target) {
        let _ = fs::rename(&backup, target);
        let _ = fs::remove_dir_all(&stage);
        return Err(format!("Could not install Panda Reader {version}: {error}"));
    }
    if let Err(error) = Command::new("/usr/bin/open").arg("-n").arg(target).spawn() {
        let _ = fs::remove_dir_all(target);
        let _ = fs::rename(&backup, target);
        let _ = fs::remove_dir_all(&stage);
        return Err(format!("Could not relaunch Panda Reader: {error}"));
    }
    let _ = fs::remove_dir_all(backup);
    let _ = fs::remove_dir_all(stage);
    Ok(())
}

#[cfg(target_os = "linux")]
fn apply_linux_appimage(
    package: &Path,
    target: &Path,
    version: &semver::Version,
) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "Could not locate the AppImage folder.".to_owned())?;
    let file_name = target
        .file_name()
        .ok_or_else(|| "The AppImage path has no file name.".to_owned())?;
    let staged = parent.join(format!(".panda-reader-update-{}", std::process::id()));
    fs::copy(package, &staged).map_err(|error| format!("Could not stage the AppImage: {error}"))?;
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
        .map_err(|error| error.to_string())?;
    let backup = parent.join(format!(".panda-reader-backup-{}", std::process::id()));
    fs::rename(target, &backup).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(&staged, target) {
        let _ = fs::rename(&backup, target);
        let _ = fs::remove_file(&staged);
        return Err(format!("Could not install AppImage {version}: {error}"));
    }
    let launch = Command::new(parent.join(file_name)).spawn();
    if let Err(error) = launch {
        let _ = fs::remove_file(target);
        let _ = fs::rename(&backup, target);
        return Err(format!("Could not relaunch Panda Reader: {error}"));
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn apply_windows_setup(_: &Path, _: &Path, _: &semver::Version) -> Result<(), String> {
    Err("Windows updater invoked on an unsupported platform.".into())
}

#[cfg(not(target_os = "macos"))]
fn apply_mac_bundle(_: &Path, _: &Path, _: &semver::Version) -> Result<(), String> {
    Err("macOS updater invoked on an unsupported platform.".into())
}

#[cfg(not(target_os = "linux"))]
fn apply_linux_appimage(_: &Path, _: &Path, _: &semver::Version) -> Result<(), String> {
    Err("Linux updater invoked on an unsupported platform.".into())
}
