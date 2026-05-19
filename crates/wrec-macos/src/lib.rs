use wrec_core::{
    CaptureSourceKind, CaptureTarget, RecorderEngine, RecorderError, RecorderSettings,
    RecordingSession, Result,
};

#[derive(Debug, Clone)]
pub enum RecorderEvent {
    Log(String),
    Failed(String),
    Exited { success: bool, status: String },
}

#[derive(Default)]
pub struct MacosRecorder {
    active: Option<RecordingSession>,
    events: Option<std::sync::mpsc::Sender<RecorderEvent>>,
}

impl MacosRecorder {
    pub fn new(events: std::sync::mpsc::Sender<RecorderEvent>) -> Self {
        Self {
            events: Some(events),
            ..Self::default()
        }
    }

    fn emit(&self, event: RecorderEvent) {
        if let Some(events) = &self.events {
            let _ = events.send(event);
        }
    }
}

impl RecorderEngine for MacosRecorder {
    fn list_targets(&self) -> Result<Vec<CaptureTarget>> {
        platform::list_targets()
    }

    fn start(
        &mut self,
        target: CaptureTarget,
        settings: RecorderSettings,
    ) -> Result<RecordingSession> {
        self.emit(RecorderEvent::Log(format!(
            "starting capture: {} ({:?})",
            target.name, target.kind
        )));
        let session = platform::start_recording(target, settings, self.events.clone())?;
        self.active = Some(session.clone());
        self.emit(RecorderEvent::Log(format!(
            "recording output: {}",
            session.output_path.display()
        )));
        Ok(session)
    }

    fn stop(&mut self) -> Result<()> {
        self.emit(RecorderEvent::Log("stopping recording".to_string()));
        platform::stop_recording()?;
        self.active = None;
        self.emit(RecorderEvent::Log("recording stopped".to_string()));
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::sync::{Mutex, OnceLock};

    static CHILD: OnceLock<Mutex<Option<std::process::Child>>> = OnceLock::new();

    pub fn list_targets() -> Result<Vec<CaptureTarget>> {
        use std::process::Command;

        let output = Command::new("xcrun")
            .arg("swift")
            .arg(helper_path())
            .arg("--list")
            .output()
            .map_err(|err| RecorderError::Backend(format!("failed to list targets: {err}")))?;

        if !output.status.success() {
            return Err(RecorderError::Backend(format!(
                "target listing failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let mut targets = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let mut parts = line.splitn(3, '\t');
            let kind = match parts.next() {
                Some("display") => CaptureSourceKind::Display,
                Some("window") => CaptureSourceKind::Window,
                _ => continue,
            };
            let Some(id) = parts.next().and_then(|id| id.parse::<u64>().ok()) else {
                continue;
            };
            let name = parts.next().unwrap_or("Unknown").to_string();
            targets.push(CaptureTarget { id, name, kind });
        }

        if targets.is_empty() {
            targets.push(CaptureTarget {
                id: 0,
                name: "Main Display".to_string(),
                kind: CaptureSourceKind::Display,
            });
        }
        Ok(targets)
    }

    pub fn start_recording(
        target: CaptureTarget,
        settings: RecorderSettings,
        events: Option<std::sync::mpsc::Sender<RecorderEvent>>,
    ) -> Result<RecordingSession> {
        use std::process::{Command, Stdio};

        let child_slot = CHILD.get_or_init(|| Mutex::new(None));
        if child_slot.lock().unwrap().is_some() {
            return Err(RecorderError::Backend("recording is already active".into()));
        }

        std::fs::create_dir_all(&settings.output_dir)
            .map_err(|err| RecorderError::Backend(err.to_string()))?;

        let filename = format!("wrec-{}.mov", chrono_like_timestamp());
        let output_path = settings.output_dir.join(filename);
        let helper = helper_path();

        // Temporary v0 native bridge: run a tiny Swift helper that uses
        // ScreenCaptureKit + SCRecordingOutput. The frame path stays inside
        // Apple's native stack; Rust never receives/copies pixels.
        let mut child = Command::new("xcrun")
            .arg("swift")
            .arg(helper)
            .arg(&output_path)
            .arg(settings.fps.as_u32().to_string())
            .arg(if settings.include_cursor {
                "true"
            } else {
                "false"
            })
            .arg(match target.kind {
                CaptureSourceKind::Display => "display",
                CaptureSourceKind::Window => "window",
            })
            .arg(target.id.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| RecorderError::Backend(format!("failed to start helper: {err}")))?;

        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || forward_helper_stderr(stderr, events));
        }

        tracing::info!(?target, ?settings, ?output_path, "started recording helper");
        *child_slot.lock().unwrap() = Some(child);
        Ok(RecordingSession { output_path })
    }

    pub fn stop_recording() -> Result<()> {
        use std::io::Write;

        let child_slot = CHILD.get_or_init(|| Mutex::new(None));
        let Some(mut child) = child_slot.lock().unwrap().take() else {
            return Ok(());
        };

        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(b"stop\n");
        }

        let status = child
            .wait()
            .map_err(|err| RecorderError::Backend(format!("failed waiting for helper: {err}")))?;
        if !status.success() {
            return Err(RecorderError::Backend(format!(
                "recording helper exited with {status}"
            )));
        }
        Ok(())
    }

    fn forward_helper_stderr(
        stderr: std::process::ChildStderr,
        events: Option<std::sync::mpsc::Sender<RecorderEvent>>,
    ) {
        for line in BufReader::new(stderr)
            .lines()
            .map_while(std::result::Result::ok)
        {
            eprintln!("{line}");
            emit(
                &events,
                if line.contains("recording failed") {
                    RecorderEvent::Failed(line)
                } else {
                    RecorderEvent::Log(line)
                },
            );
        }

        let child_slot = CHILD.get_or_init(|| Mutex::new(None));
        let Ok(mut child) = child_slot.lock() else {
            return;
        };
        let Some(status) = child
            .as_mut()
            .and_then(|child| child.try_wait().ok())
            .flatten()
        else {
            return;
        };
        *child = None;
        emit(
            &events,
            RecorderEvent::Exited {
                success: status.success(),
                status: status.to_string(),
            },
        );
    }

    fn emit(events: &Option<std::sync::mpsc::Sender<RecorderEvent>>, event: RecorderEvent) {
        if let Some(events) = events {
            let _ = events.send(event);
        }
    }

    fn helper_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("native")
            .join("wrec_helper.swift")
    }

    fn chrono_like_timestamp() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        secs.to_string()
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub fn list_targets() -> Result<Vec<CaptureTarget>> {
        Err(RecorderError::Backend("wrec only supports macOS".into()))
    }

    pub fn start_recording(
        _target: CaptureTarget,
        _settings: RecorderSettings,
    ) -> Result<RecordingSession> {
        Err(RecorderError::Backend("wrec only supports macOS".into()))
    }

    pub fn stop_recording() -> Result<()> {
        Err(RecorderError::Backend("wrec only supports macOS".into()))
    }
}
