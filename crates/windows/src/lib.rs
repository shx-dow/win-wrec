use domain::{
    CaptureTarget, RecorderEngine, RecorderError, RecorderEvent,
    RecorderSettings, RecordingSession, Result,
};

#[cfg(target_os = "windows")]
use domain::RecorderMetrics;

static LAST_SESSION_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Default)]
pub struct WindowsRecorder {
    active: Option<RecordingSession>,
    events: Option<std::sync::mpsc::Sender<RecorderEvent>>,
    capture: Option<CaptureState>,
}

struct CaptureState {
    #[allow(dead_code)]
    session_id: u64,
    #[allow(dead_code)]
    output_path: std::path::PathBuf,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl WindowsRecorder {
    pub fn new(events: std::sync::mpsc::Sender<RecorderEvent>) -> Self {
        Self {
            active: None,
            events: Some(events),
            capture: None,
        }
    }

    fn emit(&self, event: RecorderEvent) {
        if let Some(events) = &self.events {
            let _ = events.send(event);
        }
    }

    fn emit_log(&self, session_id: Option<u64>, message: impl Into<String>) {
        self.emit(RecorderEvent::Log {
            session_id,
            message: message.into(),
        });
    }

    fn next_session_id(&self) -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now_micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros().min(u64::MAX as u128) as u64)
            .unwrap_or_default();
        loop {
            let current = LAST_SESSION_ID.load(std::sync::atomic::Ordering::Relaxed);
            let next = now_micros.max(current.saturating_add(1));
            if LAST_SESSION_ID
                .compare_exchange(
                    current,
                    next,
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                )
                .is_ok()
            {
                return next;
            }
        }
    }

    fn recording_output_path(&self, settings: &RecorderSettings) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let filename = format!("wrec-{}.mp4", secs);
        settings.output_dir.join(filename)
    }
}

impl RecorderEngine for WindowsRecorder {
    fn list_targets(&self) -> Result<Vec<CaptureTarget>> {
        platform::list_targets()
    }

    fn start(
        &mut self,
        target: CaptureTarget,
        settings: RecorderSettings,
    ) -> Result<RecordingSession> {
        let session_id = self.next_session_id();
        let output_path = self.recording_output_path(&settings);
        self.emit(RecorderEvent::Starting {
            session_id,
            target: target.clone(),
            settings: settings.clone(),
            output_path: output_path.clone(),
        });

        let session = match platform::start_capture(
            session_id,
            &target,
            &settings,
            &output_path,
            self.events.clone(),
        ) {
            Ok((handle, shutdown)) => {
                self.capture = Some(CaptureState {
                    session_id,
                    output_path: output_path.clone(),
                    shutdown,
                    handle: Some(handle),
                });
                RecordingSession {
                    id: session_id,
                    output_path,
                }
            }
            Err(err) => {
                self.emit(RecorderEvent::Failed {
                    session_id: Some(session_id),
                    message: err.to_string(),
                });
                return Err(err);
            }
        };

        self.active = Some(session.clone());
        self.emit_log(
            Some(session.id),
            format!("recording output: {}", session.output_path.display()),
        );
        Ok(session)
    }

    fn stop(&mut self) -> Result<()> {
        let session_id = self.active.as_ref().map(|s| s.id);
        self.emit_log(session_id, "stopping recording");

        if let Some(capture) = self.capture.take() {
            capture.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(handle) = capture.handle {
                let _ = handle.join();
            }
        }

        self.active = None;
        self.emit_log(session_id, "recording stopped");
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        if self.capture.is_some() {
            platform::set_paused(true);
            let session_id = self.active.as_ref().map(|s| s.id);
            self.emit_log(session_id, "recording paused");
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        if self.capture.is_some() {
            platform::set_paused(false);
            let session_id = self.active.as_ref().map(|s| s.id);
            self.emit_log(session_id, "recording resumed");
        }
        Ok(())
    }
}

impl Drop for WindowsRecorder {
    fn drop(&mut self) {
        if let Some(capture) = self.capture.take() {
            capture.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::sync::{atomic::AtomicBool, mpsc, Arc};

    mod dxgi;
    mod audio;
    mod encoder;

    use dxgi::DxgiCapture;
    use audio::WasapiCapture;
    use encoder::MfEncoder;

    static PAUSED: AtomicBool = AtomicBool::new(false);

    pub fn set_paused(paused: bool) {
        PAUSED.store(paused, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn list_targets() -> Result<Vec<CaptureTarget>> {
        dxgi::enumerate_targets()
    }

    pub fn start_capture(
        session_id: u64,
        target: &CaptureTarget,
        settings: &RecorderSettings,
        output_path: &std::path::PathBuf,
        events: Option<mpsc::Sender<RecorderEvent>>,
    ) -> Result<(std::thread::JoinHandle<()>, Arc<AtomicBool>)> {
        std::fs::create_dir_all(&settings.output_dir)
            .map_err(|e| RecorderError::Backend(e.to_string()))?;

        let paused = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let paused_clone = paused.clone();
        let target_clone = target.clone();
        let settings_clone = settings.clone();
        let output_path_clone = output_path.clone();

        let handle = std::thread::Builder::new()
            .name("wrec-capture".into())
            .spawn(move || {
                if let Err(e) = run_capture_thread(
                    session_id,
                    &target_clone,
                    &settings_clone,
                    &output_path_clone,
                    &shutdown_clone,
                    &paused_clone,
                    &events,
                ) {
                    emit_event(&events, RecorderEvent::Failed {
                        session_id: Some(session_id),
                        message: e.to_string(),
                    });
                    tracing::error!(?e, "capture thread failed");
                }
            })
            .map_err(|e| RecorderError::Backend(format!("failed to spawn capture thread: {e}")))?;

        Ok((handle, shutdown))
    }

    fn run_capture_thread(
        session_id: u64,
        target: &CaptureTarget,
        settings: &RecorderSettings,
        output_path: &std::path::PathBuf,
        shutdown: &AtomicBool,
        paused: &AtomicBool,
        events: &Option<mpsc::Sender<RecorderEvent>>,
    ) -> Result<()> {
        unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            )
            .ok()
            .map_err(|e| RecorderError::Backend(format!("CoInitializeEx failed: {e}")))?;
        }

        let mut dxgi = DxgiCapture::new(target)
            .map_err(|e| RecorderError::Backend(format!("DXGI init: {e}")))?;
        let mut audio = WasapiCapture::new(settings.include_system_audio)
            .map_err(|e| RecorderError::Backend(format!("WASAPI init: {e}")))?;
        let video_media_type = dxgi.media_type();
        let audio_media_type = audio.media_type();
        let mut encoder = MfEncoder::new(
            output_path,
            &video_media_type,
            audio_media_type.as_ref(),
            settings.fps.as_u32(),
            settings.quality,
            settings.codec,
        ).map_err(|e| RecorderError::Backend(format!("encoder init: {e}")))?;

        let target_fps = settings.fps.as_u32();
        let frame_duration = std::time::Duration::from_micros(1_000_000 / target_fps as u64);
        let mut next_frame_time = std::time::Instant::now();

        emit_event(events, RecorderEvent::Log {
            session_id: Some(session_id),
            message: format!(
                "capture started: {}x{} @ {}fps, audio={}",
                dxgi.width(), dxgi.height(), target_fps,
                settings.include_system_audio,
            ),
        });

        emit_event(events, RecorderEvent::Log {
            session_id: Some(session_id),
            message: "capture started OK".into(),
        });

        let metric_start = std::time::Instant::now();
        let mut frame_count: u64 = 0;

        while !shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            let now = std::time::Instant::now();
            if now < next_frame_time {
                std::thread::sleep(next_frame_time - now);
            }
            next_frame_time += frame_duration;

            if paused.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = audio.poll();
                continue;
            }

            if let Some(audio_samples) = audio.poll() {
                if let Err(e) = encoder.write_audio(&audio_samples) {
                    tracing::error!(?e, "audio write failed");
                }
            } else if settings.include_system_audio {
                let ch = audio.channels() as u64;
                let sr = audio.sample_rate() as u64;
                if ch > 0 && sr > 0 {
                    let frame_period_ns = 1_000_000_000 / target_fps as u64;
                    let num_frames = (sr * frame_period_ns / 1_000_000_000).max(1) as usize;
                    let silence = vec![0u8; num_frames * ch as usize * 2];
                    let dummy = audio::AudioSamples {
                        data: silence,
                        sample_rate: sr as u32,
                        channels: ch as u16,
                        bits_per_sample: 16,
                    };
                    if let Err(e) = encoder.write_audio(&dummy) {
                        tracing::error!(?e, "silence write failed");
                    }
                }
            }

            match dxgi.acquire_frame() {
                Ok(frame_data) => {
                    if let Err(e) = encoder.write_video(&frame_data) {
                        tracing::error!(?e, "video write failed");
                    }
                    dxgi.release_frame();
                    frame_count += 1;
                }
                Err(e) => {
                    if dxgi.is_timeout(&e) {
                        continue;
                    }
                    tracing::error!(?e, "DXGI frame acquire failed");
                    break;
                }
            }

            let elapsed = metric_start.elapsed().as_secs();
            if elapsed > 0 && frame_count % (target_fps as u64 * 5) == 0 {
                let metadata = std::fs::metadata(output_path).ok();
                let output_bytes = metadata.map(|m| m.len()).unwrap_or(0);
                let estimated_bitrate = if elapsed > 0 {
                    output_bytes as f32 * 8.0 / elapsed as f32 / 1_000_000.0
                } else {
                    0.0
                };
                emit_event(events, RecorderEvent::Metrics {
                    session_id,
                    metrics: RecorderMetrics {
                        elapsed_secs: elapsed,
                        output_bytes,
                        estimated_bitrate_mbps: estimated_bitrate,
                    },
                });
            }
        }

        tracing::info!(frames = frame_count, "capture thread stopping");
        emit_event(events, RecorderEvent::Log {
            session_id: Some(session_id),
            message: format!("captured {frame_count} frames"),
        });

        drop(dxgi);
        drop(audio);
        encoder.finalize().map_err(|e| RecorderError::Backend(format!("encoder finalize: {e}")))?;

        unsafe { windows::Win32::System::Com::CoUninitialize(); }

        let metadata = std::fs::metadata(output_path).ok();
        let output_bytes = metadata.map(|m| m.len()).unwrap_or(0);
        let elapsed = metric_start.elapsed().as_secs();
        emit_event(events, RecorderEvent::Exited {
            session_id,
            success: true,
            status: format!(
                "recording finished frames={frame_count} size={output_bytes} elapsed={elapsed}s"
            ),
        });

        Ok(())
    }

    fn emit_event(events: &Option<mpsc::Sender<RecorderEvent>>, event: RecorderEvent) {
        if let Some(events) = events {
            let _ = events.send(event);
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};

    pub fn set_paused(_paused: bool) {}

    pub fn list_targets() -> Result<Vec<CaptureTarget>> {
        Err(RecorderError::Backend("wrec only supports Windows".into()))
    }

    pub fn start_capture(
        _session_id: u64,
        _target: &CaptureTarget,
        _settings: &RecorderSettings,
        _output_path: &std::path::PathBuf,
        _events: Option<std::sync::mpsc::Sender<RecorderEvent>>,
    ) -> Result<(std::thread::JoinHandle<()>, Arc<AtomicBool>)> {
        Err(RecorderError::Backend("wrec only supports Windows".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_recorder_has_no_active_session() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let recorder = WindowsRecorder::new(tx);

        assert!(recorder.active.is_none());
        assert!(recorder.capture.is_none());
    }
}
