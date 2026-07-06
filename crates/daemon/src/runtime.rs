use control::AgentError;
use domain::{
    CaptureTarget, RecorderEngine, RecorderError, RecorderEvent, ScreenRecordingPermissionStatus,
};
use std::sync::mpsc;

#[cfg(target_os = "macos")]
use macos::MacosRecorder;
#[cfg(target_os = "windows")]
use windows_recorder::WindowsRecorder;

pub(crate) trait RecordingRuntime: Clone + Send + Sync + 'static {
    type Engine: RecorderEngine + Send + 'static;

    fn list_targets(&self) -> Result<Vec<CaptureTarget>, AgentError>;
    fn screen_recording_permission_status(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError>;
    fn request_screen_recording_permission(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError>;
    fn new_engine(&self, events: mpsc::Sender<RecorderEvent>) -> Self::Engine;
}

#[cfg(target_os = "macos")]
pub(crate) type PlatformRuntime = MacosRuntime;
#[cfg(target_os = "windows")]
pub(crate) type PlatformRuntime = WindowsRuntime;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) type PlatformRuntime = UnsupportedRuntime;

#[cfg(target_os = "macos")]
#[derive(Clone, Default)]
pub(crate) struct MacosRuntime;

#[cfg(target_os = "macos")]
impl RecordingRuntime for MacosRuntime {
    type Engine = MacosRecorder;

    fn list_targets(&self) -> Result<Vec<CaptureTarget>, AgentError> {
        let (tx, _rx) = mpsc::channel();
        MacosRecorder::new(tx).list_targets().map_err(|err| AgentError {
            code: "target_listing_failed".into(),
            message: err.to_string(),
            recoverable: true,
            next: "Run `wrec targets --json` again; if this repeats, check Screen Recording permission and ~/.wrec/daemon.log.".into(),
        })
    }

    fn screen_recording_permission_status(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError> {
        let (tx, _rx) = mpsc::channel();
        MacosRecorder::new(tx)
            .screen_recording_permission_status()
            .map_err(permission_error)
    }

    fn request_screen_recording_permission(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError> {
        let (tx, _rx) = mpsc::channel();
        MacosRecorder::new(tx)
            .request_screen_recording_permission()
            .map_err(permission_error)
    }

    fn new_engine(&self, events: mpsc::Sender<RecorderEvent>) -> Self::Engine {
        MacosRecorder::new(events)
    }
}

#[cfg(target_os = "windows")]
#[derive(Clone, Default)]
pub(crate) struct WindowsRuntime;

#[cfg(target_os = "windows")]
impl RecordingRuntime for WindowsRuntime {
    type Engine = WindowsRecorder;

    fn list_targets(&self) -> Result<Vec<CaptureTarget>, AgentError> {
        let (tx, _rx) = mpsc::channel();
        WindowsRecorder::new(tx).list_targets().map_err(|err| AgentError {
            code: "target_listing_failed".into(),
            message: err.to_string(),
            recoverable: true,
            next: "Run `wrec targets --json` again; if this repeats, check whether Windows Graphics Capture is supported and inspect %LOCALAPPDATA%\\Wrec\\daemon.log.".into(),
        })
    }

    fn screen_recording_permission_status(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError> {
        let (tx, _rx) = mpsc::channel();
        WindowsRecorder::new(tx)
            .screen_recording_permission_status()
            .map_err(permission_error)
    }

    fn request_screen_recording_permission(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError> {
        let (tx, _rx) = mpsc::channel();
        WindowsRecorder::new(tx)
            .request_screen_recording_permission()
            .map_err(permission_error)
    }

    fn new_engine(&self, events: mpsc::Sender<RecorderEvent>) -> Self::Engine {
        WindowsRecorder::new(events)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[derive(Clone, Default)]
pub(crate) struct UnsupportedRuntime;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) struct UnsupportedRecorder;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl RecordingRuntime for UnsupportedRuntime {
    type Engine = UnsupportedRecorder;

    fn list_targets(&self) -> Result<Vec<CaptureTarget>, AgentError> {
        Err(unsupported_platform_error())
    }

    fn screen_recording_permission_status(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError> {
        Err(unsupported_platform_error())
    }

    fn request_screen_recording_permission(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError> {
        Err(unsupported_platform_error())
    }

    fn new_engine(&self, _events: mpsc::Sender<RecorderEvent>) -> Self::Engine {
        UnsupportedRecorder
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl RecorderEngine for UnsupportedRecorder {
    fn list_targets(&self) -> domain::Result<Vec<CaptureTarget>> {
        Err(RecorderError::Backend(
            "wrec only supports macOS and Windows".into(),
        ))
    }

    fn start(
        &mut self,
        _target: CaptureTarget,
        _settings: domain::RecorderSettings,
    ) -> domain::Result<domain::RecordingSession> {
        Err(RecorderError::Backend(
            "wrec only supports macOS and Windows".into(),
        ))
    }

    fn pause(&mut self) -> domain::Result<()> {
        Err(RecorderError::Backend(
            "wrec only supports macOS and Windows".into(),
        ))
    }

    fn resume(&mut self) -> domain::Result<()> {
        Err(RecorderError::Backend(
            "wrec only supports macOS and Windows".into(),
        ))
    }

    fn stop(&mut self) -> domain::Result<()> {
        Err(RecorderError::Backend(
            "wrec only supports macOS and Windows".into(),
        ))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn unsupported_platform_error() -> AgentError {
    AgentError {
        code: "unsupported_platform".into(),
        message: "wrec only supports macOS and Windows".into(),
        recoverable: false,
        next: "Run wrec on macOS 15+ or Windows 10 1903+.".into(),
    }
}

fn permission_error(error: RecorderError) -> AgentError {
    match error {
        RecorderError::MissingScreenRecordingPermission => AgentError {
            code: "screen_recording_permission_missing".into(),
            message: "screen recording permission is not granted".into(),
            recoverable: true,
            next: "Grant Screen Recording permission, then retry.".into(),
        },
        RecorderError::Backend(message) if message.contains("capture-engine") => AgentError {
            code: "capture_engine_missing".into(),
            message: format!("backend error: {message}"),
            recoverable: true,
            next: "Build the daemon through Cargo or install the full wrec runtime so daemon and capture-engine are present together.".into(),
        },
        error => AgentError {
            code: "screen_recording_permission_failed".into(),
            message: error.to_string(),
            recoverable: true,
            next: "Fix the backend error above, then retry the permission check.".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_permission_maps_to_permission_missing_code() {
        let error = permission_error(RecorderError::MissingScreenRecordingPermission);

        assert_eq!(error.code, "screen_recording_permission_missing");
        assert!(error.recoverable);
    }

    #[test]
    fn capture_engine_backend_errors_map_to_capture_engine_missing() {
        let error = permission_error(RecorderError::Backend(
            "capture-engine binary not found".into(),
        ));

        assert_eq!(error.code, "capture_engine_missing");
        assert!(error.message.contains("capture-engine binary not found"));
    }

    #[test]
    fn other_errors_map_to_permission_failed() {
        let error = permission_error(RecorderError::Backend("boom".into()));

        assert_eq!(error.code, "screen_recording_permission_failed");
        assert_eq!(error.message, "backend error: boom");
    }
}
