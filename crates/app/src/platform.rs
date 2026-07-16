#[cfg(target_os = "windows")]
use std::ffi::c_void;
#[cfg(not(target_os = "windows"))]
use std::fs;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(not(target_os = "windows"))]
const CLI_INSTALLER_URL: &str = "https://wrec-beta.vercel.app/install";
#[cfg(not(target_os = "windows"))]
const MANAGED_CLI_MARKER: &str = "# managed by wrec";
#[cfg(not(target_os = "windows"))]
const INSTALLED_BIN: &str = "/usr/local/bin/wrec";
#[cfg(not(target_os = "windows"))]
const INSTALLED_CLI: &str = "/usr/local/lib/wrec/wrec";
#[cfg(not(target_os = "windows"))]
const INSTALLED_DAEMON: &str = "/usr/local/lib/wrec/daemon";
#[cfg(not(target_os = "windows"))]
const INSTALLED_CAPTURE_ENGINE: &str = "/usr/local/lib/wrec/capture-engine";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CliInstallStatus {
    NotInstalled,
    Installed,
    NeedsUpdate,
    Conflict,
}

impl CliInstallStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NotInstalled => "Not installed",
            Self::Installed => "Installed",
            Self::NeedsUpdate => "Update available",
            Self::Conflict => "Path conflict",
        }
    }
}

pub(crate) fn choose_output_dir() -> Option<PathBuf> {
    choose_output_dir_impl()
}

#[cfg(target_os = "macos")]
fn choose_output_dir_impl() -> Option<PathBuf> {
    let output = Command::new("osascript")
        .args([
            "-e",
            r#"POSIX path of (choose folder with prompt "Choose recording folder")"#,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(target_os = "windows")]
fn choose_output_dir_impl() -> Option<PathBuf> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-STA",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { $d.SelectedPath }",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn choose_output_dir_impl() -> Option<PathBuf> {
    None
}

pub(crate) fn open_path(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        return Command::new("open").arg(path).spawn().map(|_| ());
    }

    #[cfg(target_os = "windows")]
    {
        return Command::new("explorer").arg(path).spawn().map(|_| ());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Command::new("xdg-open").arg(path).spawn().map(|_| ())
    }
}

pub(crate) fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        return Command::new("open").arg(url).spawn().map(|_| ());
    }

    #[cfg(target_os = "windows")]
    {
        return Command::new("explorer").arg(url).spawn().map(|_| ());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Command::new("xdg-open").arg(url).spawn().map(|_| ())
    }
}

pub(crate) fn set_window_capture_excluded(
    window: &mut gpui::Window,
    excluded: bool,
) -> Result<(), String> {
    set_window_capture_excluded_impl(window, excluded)
}

#[cfg(target_os = "windows")]
fn set_window_capture_excluded_impl(
    window: &mut gpui::Window,
    excluded: bool,
) -> Result<(), String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE},
    };

    let handle = window
        .window_handle()
        .map_err(|err| format!("window handle unavailable: {err}"))?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return Err("window is not backed by a Win32 HWND".into());
    };

    let affinity = if excluded {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    unsafe { SetWindowDisplayAffinity(HWND(handle.hwnd.get() as *mut c_void), affinity) }
        .map_err(|err| format!("SetWindowDisplayAffinity failed: {err}"))
}

#[cfg(not(target_os = "windows"))]
fn set_window_capture_excluded_impl(
    _window: &mut gpui::Window,
    _excluded: bool,
) -> Result<(), String> {
    Ok(())
}

pub(crate) fn cli_install_status() -> CliInstallStatus {
    cli_install_status_impl()
}

#[cfg(not(target_os = "windows"))]
fn cli_install_status_impl() -> CliInstallStatus {
    let installed_bin = Path::new(INSTALLED_BIN);
    if !installed_bin.exists() {
        return CliInstallStatus::NotInstalled;
    }
    if !managed_cli_bin(installed_bin) {
        return CliInstallStatus::Conflict;
    }

    if [INSTALLED_CLI, INSTALLED_DAEMON, INSTALLED_CAPTURE_ENGINE]
        .into_iter()
        .all(|path| Path::new(path).is_file())
    {
        CliInstallStatus::Installed
    } else {
        CliInstallStatus::NeedsUpdate
    }
}

pub(crate) fn cli_install_command() -> Option<String> {
    cli_install_command_impl()
}

#[cfg(not(target_os = "windows"))]
fn cli_install_command_impl() -> Option<String> {
    let version = current_app_bundle_path()
        .as_deref()
        .filter(|app| !is_dev_app(app))
        .map(|_| env!("CARGO_PKG_VERSION"));

    Some(match version {
        Some(version) => format!("curl -fsSL {CLI_INSTALLER_URL} | WREC_VERSION={version} sh"),
        None => format!("curl -fsSL {CLI_INSTALLER_URL} | sh"),
    })
}

#[cfg(target_os = "windows")]
fn cli_install_status_impl() -> CliInstallStatus {
    let Some(runtime_dir) = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|dir| dir.join("Wrec"))
    else {
        return CliInstallStatus::NotInstalled;
    };

    if runtime_dir.exists() && !runtime_dir.is_dir() {
        return CliInstallStatus::Conflict;
    }

    if ["wrec.exe", "daemon.exe", "capture-engine.exe"]
        .into_iter()
        .all(|name| runtime_dir.join(name).is_file())
    {
        CliInstallStatus::Installed
    } else if runtime_dir.exists() {
        CliInstallStatus::NeedsUpdate
    } else {
        CliInstallStatus::NotInstalled
    }
}

#[cfg(target_os = "windows")]
fn cli_install_command_impl() -> Option<String> {
    None
}

#[cfg(not(target_os = "windows"))]
fn current_app_bundle_path() -> Option<PathBuf> {
    std::env::current_exe().ok()?.ancestors().find_map(|path| {
        let name = path.file_name()?.to_str()?;
        name.ends_with(".app").then(|| path.to_path_buf())
    })
}

#[cfg(not(target_os = "windows"))]
fn is_dev_app(app: &Path) -> bool {
    let name_is_dev = app
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains("Dev"));
    let info_is_dev = fs::read_to_string(app.join("Contents").join("Info.plist"))
        .is_ok_and(|info| info.contains("app.wrec.dev") || info.contains("Wrec Dev"));

    name_is_dev || info_is_dev
}

#[cfg(not(target_os = "windows"))]
fn managed_cli_bin(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|contents| contents.contains(MANAGED_CLI_MARKER))
}
