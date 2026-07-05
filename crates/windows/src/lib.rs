use domain::{
    CaptureTarget, RecorderEngine, RecorderError, RecorderEvent,
    RecorderSettings, RecordingSession, Result,
};
use std::sync::{atomic::AtomicBool, mpsc, Arc};

pub struct WindowsRecorder {
    active: Option<RecordingSession>,
    events: Option<mpsc::Sender<RecorderEvent>>,
    capture: Option<CaptureState>,
}

#[allow(dead_code)]
struct CaptureState {
    session_id: u64,
    output_path: std::path::PathBuf,
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl WindowsRecorder {
    pub fn new(events: mpsc::Sender<RecorderEvent>) -> Self {
        Self { active: None, events: Some(events), capture: None }
    }

    fn emit(&self, event: RecorderEvent) {
        if let Some(events) = &self.events {
            let _ = events.send(event);
        }
    }
}

impl RecorderEngine for WindowsRecorder {
    fn list_targets(&self) -> Result<Vec<CaptureTarget>> {
        Err(RecorderError::Backend("Windows capture not yet implemented".into()))
    }

    fn start(&mut self, _target: CaptureTarget, _settings: RecorderSettings) -> Result<RecordingSession> {
        Err(RecorderError::Backend("Windows capture not yet implemented".into()))
    }

    fn stop(&mut self) -> Result<()> { Ok(()) }
    fn pause(&mut self) -> Result<()> { Ok(()) }
    fn resume(&mut self) -> Result<()> { Ok(()) }
}

impl Drop for WindowsRecorder {
    fn drop(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_recorder_has_no_active_session() {
        let (tx, _rx) = mpsc::channel();
        let recorder = WindowsRecorder::new(tx);
        assert!(recorder.active.is_none());
        assert!(recorder.capture.is_none());
    }
}
