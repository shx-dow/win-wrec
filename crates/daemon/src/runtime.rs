use control::AgentError;
use domain::{
    CaptureTarget, RecorderEngine, RecorderEvent, ScreenRecordingPermissionStatus,
};
use std::sync::mpsc;

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

#[derive(Clone, Default)]
pub(crate) struct WindowsRuntime;

impl RecordingRuntime for WindowsRuntime {
    type Engine = windows_recorder::WindowsRecorder;

    fn list_targets(&self) -> Result<Vec<CaptureTarget>, AgentError> {
        let (tx, _rx) = mpsc::channel();
        windows_recorder::WindowsRecorder::new(tx)
            .list_targets()
            .map_err(|err| AgentError {
                code: "target_listing_failed".into(),
                message: err.to_string(),
                recoverable: true,
                next: "Run `wrec targets --json` again; if this repeats, check that a display is available.".into(),
            })
    }

    fn screen_recording_permission_status(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError> {
        Ok(ScreenRecordingPermissionStatus::Granted)
    }

    fn request_screen_recording_permission(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError> {
        Ok(ScreenRecordingPermissionStatus::Granted)
    }

    fn new_engine(&self, events: mpsc::Sender<RecorderEvent>) -> Self::Engine {
        windows_recorder::WindowsRecorder::new(events)
    }
}
