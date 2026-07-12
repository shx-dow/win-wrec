use crate::platform::{choose_output_dir, open_path, CliInstallStatus};
use crate::ui::{resolution_label, target_key, AppTab};
use config::{save_config as persist_config, wrec_dir, AppConfig};
use control::{
    AgentError, DaemonClient, EventLevel, JobSnapshot, JobStatus, RecordingOptions,
    StartRecordingParams, TargetSelector,
};
use domain::{
    CaptureSourceKind, CaptureTarget, Codec, FrameRate, Quality, RecorderMetrics, RecorderSettings,
    Resolution, ScreenRecordingPermissionStatus,
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

pub(crate) const GITHUB_URL: &str = "https://github.com/shivamhwp/wrec";
const MAX_LOGS: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecorderState {
    Idle,
    LoadingTargets,
    Starting,
    Recording,
    Pausing,
    Paused,
    Resuming,
    Stopping,
    Failed,
}

impl RecorderState {
    pub(crate) fn is_recording(&self) -> bool {
        matches!(self, Self::Recording)
    }
    pub(crate) fn is_paused(&self) -> bool {
        matches!(self, Self::Paused)
    }
    pub(crate) fn is_active_session(&self) -> bool {
        matches!(
            self,
            Self::Recording | Self::Pausing | Self::Paused | Self::Resuming | Self::Stopping
        )
    }
    pub(crate) fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::LoadingTargets | Self::Starting | Self::Pausing | Self::Resuming | Self::Stopping
        )
    }
}

#[derive(Debug)]
enum AppEvent {
    PermissionChecked {
        result: std::result::Result<ScreenRecordingPermissionStatus, String>,
        refresh_targets_after_granted: bool,
    },
    PermissionRequested(std::result::Result<ScreenRecordingPermissionStatus, String>),
    TargetsLoaded(std::result::Result<Vec<CaptureTarget>, String>),
    Started(std::result::Result<JobSnapshot, String>),
    JobPolled(std::result::Result<JobSnapshot, String>),
    Paused(std::result::Result<JobSnapshot, String>),
    Resumed(std::result::Result<JobSnapshot, String>),
    Stopped(std::result::Result<JobSnapshot, String>),
}

enum UiEvent {
    App(AppEvent),
}

pub(crate) struct WrecApp {
    daemon: DaemonClient,
    app_events: mpsc::Sender<UiEvent>,
    ui_receiver: mpsc::Receiver<UiEvent>,
    pub(crate) settings: RecorderSettings,
    targets: Vec<CaptureTarget>,
    pub(crate) selected_target_key: Option<String>,
    active_job_id: Option<u64>,
    active_job_event_count: usize,
    active_output_path: Option<PathBuf>,
    pub(crate) recorder_state: RecorderState,
    pub(crate) permission_status: ScreenRecordingPermissionStatus,
    pub(crate) permission_busy: bool,
    pub(crate) metrics: Option<RecorderMetrics>,
    pub(crate) status: String,
    pub(crate) cli_install_status: CliInstallStatus,
    pub(crate) active_tab: AppTab,
    pub(crate) last_recording_dir: Option<PathBuf>,
    pub(crate) show_nerd_logs: bool,
    pub(crate) logs: VecDeque<String>,
    quit_after_stop: bool,
    window_capture_excluded: bool,
    pub(crate) is_dark_mode: bool,
    pub(crate) native_window_handle: isize,
    /// Idle → Recording transitions. When true, the Record button
    /// shows a red "● Recording" label and the Stop button.
    pub(crate) show_quit_dialog: bool,
    pub(crate) show_error_dialog: bool,
    pub(crate) error_dialog_message: String,
    pub(crate) notification_message: Option<(String, f64)>,
}

impl WrecApp {
    pub(crate) fn new() -> Self {
        let config = AppConfig::load();
        let settings = config.settings.with_preset_limits();

        let (app_events, ui_receiver) = mpsc::channel();

        let mut app = Self {
            daemon: DaemonClient::new(),
            app_events,
            ui_receiver,
            settings,
            targets: Vec::new(),
            selected_target_key: config.selected_target_key,
            active_job_id: None,
            active_job_event_count: 0,
            active_output_path: None,
            recorder_state: RecorderState::Idle,
            permission_status: ScreenRecordingPermissionStatus::Unknown,
            permission_busy: false,
            metrics: None,
            status: "Idle".to_string(),
            cli_install_status: crate::platform::cli_install_status(),
            active_tab: AppTab::General,
            last_recording_dir: None,
            show_nerd_logs: config.show_nerd_logs,
            logs: VecDeque::new(),
            quit_after_stop: false,
            window_capture_excluded: false,
            is_dark_mode: false,
            native_window_handle: 0,
            show_quit_dialog: false,
            show_error_dialog: false,
            error_dialog_message: String::new(),
            notification_message: None,
        };
        app.refresh_permission_status(true);
        app
    }

    pub(crate) fn process_events(&mut self) {
        while let Ok(event) = self.ui_receiver.try_recv() {
            match event {
                UiEvent::App(event) => self.handle_app_event(event),
            }
        }
    }

    pub(crate) fn update_notification(&mut self, dt: f64) {
        if let Some((_, ref mut timer)) = self.notification_message {
            *timer -= dt;
            if *timer <= 0.0 {
                self.notification_message = None;
            }
        }
    }

    pub(crate) fn show_notification(&mut self, message: impl Into<String>) {
        self.notification_message = Some((message.into(), 4.0));
    }

    // ── Source / target ──

    pub(crate) fn set_source(&mut self, source: CaptureSourceKind) {
        self.settings.source = source;
        self.selected_target_key = None;
        self.sync_target_select();
        self.push_log(format!("source: {:?}", source));
        self.save_config();
    }

    pub(crate) fn set_target_key(&mut self, key: String) {
        self.selected_target_key = Some(key);
        if let Some(target) = self.selected_target() {
            self.push_log(format!("target: {}", target.name));
        }
        self.save_config();
    }

    pub(crate) fn set_codec(&mut self, codec: Codec) {
        self.settings.codec = codec;
        self.push_log(format!("codec: {:?}", codec));
        self.save_config();
    }

    pub(crate) fn set_quality(&mut self, quality: Quality) {
        self.settings.quality = quality;
        self.push_log(format!("quality: {:?}", quality));
        self.settings = self.settings.clone().with_preset_limits();
        self.save_config();
    }

    pub(crate) fn set_resolution(&mut self, resolution: Resolution) {
        if crate::ui::resolution_disabled(self.settings.quality, resolution) {
            return;
        }
        self.settings.resolution = resolution;
        self.settings = self.settings.clone().with_preset_limits();
        self.push_log(format!("resolution: {}", resolution_label(resolution)));
        self.save_config();
    }

    pub(crate) fn set_fps(&mut self, fps: FrameRate) {
        if crate::ui::fps_disabled(self.settings.quality, fps) {
            return;
        }
        self.settings.fps = fps;
        self.settings = self.settings.clone().with_preset_limits();
        self.push_log(format!("fps: {}", fps.as_u32()));
        self.save_config();
    }

    pub(crate) fn set_include_cursor(&mut self, include_cursor: bool) {
        self.settings.include_cursor = include_cursor;
        self.push_log(format!("cursor: {}", if include_cursor { "on" } else { "off" }));
        self.save_config();
    }

    pub(crate) fn set_include_system_audio(&mut self, include_system_audio: bool) {
        self.settings.include_system_audio = include_system_audio;
        self.push_log(format!("system audio: {}", if include_system_audio { "on" } else { "off" }));
        self.save_config();
    }

    pub(crate) fn set_hide_wrec(&mut self, hide_wrec: bool) {
        self.settings.hide_wrec = hide_wrec;
        self.push_log(format!("hide wrec: {}", if hide_wrec { "on" } else { "off" }));
        self.save_config();
    }

    pub(crate) fn set_show_nerd_logs(&mut self, show: bool) {
        self.show_nerd_logs = show;
        if !show && self.active_tab == AppTab::Nerd {
            self.active_tab = AppTab::Settings;
        }
        self.push_log(format!("nerd logs: {}", if show { "on" } else { "off" }));
        self.save_config();
    }

    pub(crate) fn set_output_dir(&mut self, path: PathBuf) {
        self.settings.output_dir = path;
        self.push_log(format!("output dir: {}", self.settings.output_dir.display()));
        self.save_config();
    }

    // ── Target helpers ──

    fn sync_target_select(&mut self) {
        let source = self.settings.source;
        let target = self
            .targets
            .iter()
            .find(|t| {
                t.kind == source
                    && self
                        .selected_target_key
                        .as_deref()
                        .map_or(true, |key| target_key(t) == key)
            })
            .or_else(|| self.targets.iter().find(|t| t.kind == source));
        self.selected_target_key = target.map(|t| target_key(t));
    }

    pub(crate) fn targets_for_source(&self) -> Vec<CaptureTarget> {
        self.targets
            .iter()
            .filter(|t| t.kind == self.settings.source)
            .cloned()
            .collect()
    }

    pub(crate) fn selected_target(&self) -> Option<CaptureTarget> {
        self.selected_target_key
            .as_ref()
            .and_then(|key| {
                self.targets
                    .iter()
                    .find(|target| target_key(target) == *key)
                    .cloned()
            })
            .or_else(|| {
                self.targets
                    .iter()
                    .find(|target| target.kind == self.settings.source)
                    .cloned()
            })
    }

    pub(crate) fn selected_target_name(&self) -> Option<String> {
        self.selected_target().map(|t| t.name)
    }

    pub(crate) fn source_label(&self) -> &'static str {
        match self.settings.source {
            CaptureSourceKind::Display => "Display",
            CaptureSourceKind::Window => "Window",
        }
    }

    // ── Recording ──

    pub(crate) fn toggle_recording(&mut self) {
        if self.recorder_state.is_busy() && !self.recorder_state.is_active_session() {
            self.push_log("recorder is busy");
            return;
        }

        if self.recorder_state.is_active_session() {
            if self.recorder_state.is_busy() {
                self.push_log("recorder is busy");
                return;
            }
            let Some(job_id) = self.active_job_id else {
                self.show_error("No active daemon job to stop");
                return;
            };
            self.submit_stop_job(job_id, "stopping recording");
            return;
        }

        if !self.permission_status.is_granted() {
            self.show_error("Screen Recording permission is required");
            return;
        }

        let Some(target) = self.selected_target() else {
            self.show_error("No capture target selected");
            return;
        };

        self.start_recording(target);
    }

    pub(crate) fn toggle_pause(&mut self) {
        match self.recorder_state {
            RecorderState::Recording => {
                let Some(job_id) = self.active_job_id else {
                    self.show_error("No active daemon job to pause");
                    return;
                };
                self.recorder_state = RecorderState::Pausing;
                self.status = "Pausing".to_string();
                self.push_log("pausing recording");
                let daemon = self.daemon.clone();
                let app_events = self.app_events.clone();
                std::thread::spawn(move || {
                    let result = daemon.pause_job(job_id).map_err(agent_error_message);
                    let _ = app_events.send(UiEvent::App(AppEvent::Paused(result)));
                });
            }
            RecorderState::Paused => {
                let Some(job_id) = self.active_job_id else {
                    self.show_error("No active daemon job to resume");
                    return;
                };
                self.recorder_state = RecorderState::Resuming;
                self.status = "Resuming".to_string();
                self.push_log("resuming recording");
                let daemon = self.daemon.clone();
                let app_events = self.app_events.clone();
                std::thread::spawn(move || {
                    let result = daemon.resume_job(job_id).map_err(agent_error_message);
                    let _ = app_events.send(UiEvent::App(AppEvent::Resumed(result)));
                });
            }
            _ => self.show_error("Recording is not ready to pause or resume"),
        }
    }

    fn start_recording(&mut self, target: CaptureTarget) {
        self.recorder_state = RecorderState::Starting;
        self.status = format!("Starting {}", target.name);
        self.metrics = None;
        self.push_log(format!("target: {}", target.name));
        if self.settings.hide_wrec {
            self.push_log("hiding Wrec from recording");
            self.set_app_window_capture_excluded(true);
        } else {
            self.set_app_window_capture_excluded(false);
        }
        self.active_job_id = None;
        self.active_job_event_count = 0;
        self.active_output_path = None;
        let daemon = self.daemon.clone();
        let app_events = self.app_events.clone();
        let settings = self.settings.clone();
        std::thread::spawn(move || {
            let result = daemon
                .ensure()
                .and_then(|_| daemon.start_recording(recording_params(target, settings)))
                .map_err(agent_error_message);
            let _ = app_events.send(UiEvent::App(AppEvent::Started(result)));
        });
    }

    fn submit_stop_job(&mut self, job_id: u64, log_message: &'static str) {
        self.recorder_state = RecorderState::Stopping;
        self.status = "Stopping".to_string();
        self.push_log(log_message);
        let daemon = self.daemon.clone();
        let app_events = self.app_events.clone();
        std::thread::spawn(move || {
            let result = daemon.stop_job(job_id).map_err(agent_error_message);
            let _ = app_events.send(UiEvent::App(AppEvent::Stopped(result)));
        });
    }

    // ── Permissions ──

    pub(crate) fn refresh_permission_status(&mut self, refresh_targets_after_granted: bool) {
        if self.permission_busy {
            return;
        }
        self.permission_busy = true;
        self.status = "Checking Screen Recording permission".to_string();
        self.push_log("checking Screen Recording permission");
        let daemon = self.daemon.clone();
        let app_events = self.app_events.clone();
        std::thread::spawn(move || {
            let result = daemon
                .ensure()
                .and_then(|_| daemon.screen_recording_permission_status())
                .map_err(agent_error_message);
            let _ = app_events.send(UiEvent::App(AppEvent::PermissionChecked {
                result,
                refresh_targets_after_granted,
            }));
        });
    }

    pub(crate) fn request_screen_recording_permission(&mut self) {
        if self.permission_busy {
            return;
        }
        self.permission_busy = true;
        self.status = "Requesting Screen Recording permission".to_string();
        self.push_log("requesting Screen Recording permission");
        let daemon = self.daemon.clone();
        let app_events = self.app_events.clone();
        std::thread::spawn(move || {
            let result = daemon
                .ensure()
                .and_then(|_| daemon.request_screen_recording_permission())
                .map_err(agent_error_message);
            let _ = app_events.send(UiEvent::App(AppEvent::PermissionRequested(result)));
        });
    }

    // ── Targets loading ──

    pub(crate) fn refresh_targets(&mut self) {
        if self.recorder_state.is_busy() || self.recorder_state.is_recording() {
            return;
        }
        if !self.permission_status.is_granted() {
            self.push_log("target refresh skipped: Screen Recording permission missing");
            return;
        }
        self.recorder_state = RecorderState::LoadingTargets;
        self.status = "Loading capture targets".to_string();
        self.push_log("loading capture targets");
        let daemon = self.daemon.clone();
        let app_events = self.app_events.clone();
        std::thread::spawn(move || {
            let result = daemon
                .ensure()
                .and_then(|_| daemon.list_targets())
                .map_err(agent_error_message);
            let _ = app_events.send(UiEvent::App(AppEvent::TargetsLoaded(result)));
        });
    }

    // ── CLI ──

    pub(crate) fn refresh_cli_install_status(&mut self) {
        self.cli_install_status = crate::platform::cli_install_status();
        self.push_log(format!("cli install status: {}", self.cli_install_status.label()));
    }

    pub(crate) fn copy_cli_install_command(&mut self) {
        self.cli_install_status = crate::platform::cli_install_status();
        let Some(_command) = crate::platform::cli_install_command() else {
            self.show_error("Package Wrec as an app before installing the CLI");
            return;
        };
        // Store in clipboard via egui
        self.show_notification("CLI install command copied to clipboard");
        self.push_log("copied CLI install command");
    }

    pub(crate) fn cli_install_command(&self) -> Option<String> {
        crate::platform::cli_install_command()
    }

    // ── Output ──

    pub(crate) fn choose_output_dir_interactive(&mut self) {
        let Some(path) = choose_output_dir() else {
            self.push_log("output picker cancelled");
            return;
        };
        self.set_output_dir(path);
    }

    pub(crate) fn open_last_recording_dir(&mut self) {
        let Some(path) = self.last_recording_dir.clone() else {
            return;
        };
        match open_path(&path) {
            Ok(()) => self.push_log(format!("opened: {}", path.display())),
            Err(err) => self.show_error(format!("Could not open output folder: {err}")),
        }
    }

    pub(crate) fn open_recordings_data_dir(&mut self) {
        let path = wrec_dir();
        if let Err(err) = std::fs::create_dir_all(&path) {
            self.show_error(format!("Could not create recordings data folder: {err}"));
            return;
        }
        match open_path(&path) {
            Ok(()) => self.push_log(format!("opened recordings data folder: {}", path.display())),
            Err(err) => self.show_error(format!("Could not open recordings data folder: {err}")),
        }
    }

    pub(crate) fn open_url(&mut self, url: &str) {
        let _ = crate::platform::open_url(url);
        self.push_log(format!("opened: {url}"));
    }

    // ── Quit ──

    pub(crate) fn confirm_quit(&mut self) {
        self.show_quit_dialog = false;
        if matches!(self.recorder_state, RecorderState::Starting) {
            self.quit_after_stop = true;
            self.status = "Will quit after recording starts".to_string();
            self.push_log("waiting for recording to start before quitting");
            return;
        }
        if matches!(self.recorder_state, RecorderState::Stopping) {
            self.quit_after_stop = true;
            self.push_log("waiting for recording to stop before quitting");
            return;
        }
        if self.recorder_state.is_busy() {
            self.show_error("Recording is busy. Try again in a moment.");
            return;
        }
        let Some(job_id) = self.active_job_id else {
            self.show_error("No active daemon job to stop");
            return;
        };
        self.quit_after_stop = true;
        self.submit_stop_job(job_id, "stopping recording before quit");
    }

    // ── Window capture exclusion ──

    fn set_app_window_capture_excluded(&mut self, excluded: bool) {
        if self.window_capture_excluded == excluded {
            return;
        }
        match crate::platform::set_window_capture_excluded(self.native_window_handle, excluded) {
            Ok(()) => {
                self.window_capture_excluded = excluded;
                self.push_log(if excluded {
                    "Wrec window excluded from capture"
                } else {
                    "Wrec window capture exclusion cleared"
                });
            }
            Err(err) => {
                self.push_log(format!(
                    "Wrec window capture exclusion {} failed: {err}",
                    if excluded { "enable" } else { "clear" }
                ));
            }
        }
    }

    // ── Logging ──

    pub(crate) fn push_log(&mut self, message: impl Into<String>) {
        let message = message.into();
        tracing::info!("{message}");
        self.logs.push_back(message);
        while self.logs.len() > MAX_LOGS {
            self.logs.pop_front();
        }
    }

    fn show_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.status = message.clone();
        self.push_log(format!("error: {message}"));
        tracing::error!("{message}");
        self.error_dialog_message = message;
        self.show_error_dialog = true;
    }

    // ── Config ──

    fn save_config(&self) {
        let config = AppConfig {
            settings: self.settings.clone(),
            selected_target_key: self.selected_target_key.clone(),
            show_nerd_logs: self.show_nerd_logs,
        };
        if let Err(err) = persist_config(&config) {
            tracing::warn!("config save failed: {err}");
        }
    }

    // ── State helpers ──

    pub(crate) fn metrics_label(&self) -> String {
        self.metrics
            .as_ref()
            .map(metrics_label)
            .unwrap_or_else(zero_metrics_label)
    }

    // ── Event handling ──

    fn handle_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::PermissionChecked {
                result: Ok(status),
                refresh_targets_after_granted,
            } => {
                self.permission_busy = false;
                self.permission_status = status;
                match status {
                    ScreenRecordingPermissionStatus::Granted => {
                        self.status = "Ready".to_string();
                        self.push_log("Screen Recording permission granted");
                        if refresh_targets_after_granted {
                            self.refresh_targets();
                            return;
                        }
                    }
                    ScreenRecordingPermissionStatus::Missing => {
                        self.targets.clear();
                        self.status = "Screen Recording permission needed".to_string();
                        self.push_log("Screen Recording permission missing");
                    }
                    ScreenRecordingPermissionStatus::Unknown => {
                        self.status = "Screen Recording permission unknown".to_string();
                        self.push_log("Screen Recording permission unknown");
                    }
                }
            }
            AppEvent::PermissionChecked {
                result: Err(message),
                ..
            } => {
                self.permission_busy = false;
                self.permission_status = ScreenRecordingPermissionStatus::Unknown;
                self.show_error(message);
            }
            AppEvent::PermissionRequested(Ok(status)) => {
                self.permission_busy = false;
                self.permission_status = status;
                match status {
                    ScreenRecordingPermissionStatus::Granted => {
                        self.status = "Ready".to_string();
                        self.push_log("Screen Recording permission granted");
                        self.show_notification(
                            "Screen Recording permission granted. Refresh targets to continue.",
                        );
                    }
                    ScreenRecordingPermissionStatus::Missing => {
                        self.status = "Screen Recording permission needed".to_string();
                        self.push_log("Screen Recording permission still missing");
                        self.show_notification("Screen Recording permission is still missing");
                    }
                    ScreenRecordingPermissionStatus::Unknown => {
                        self.status = "Screen Recording permission unknown".to_string();
                        self.push_log("Screen Recording permission unknown");
                    }
                }
            }
            AppEvent::PermissionRequested(Err(message)) => {
                self.permission_busy = false;
                self.permission_status = ScreenRecordingPermissionStatus::Unknown;
                self.show_error(message);
            }
            AppEvent::TargetsLoaded(Ok(targets)) => {
                let count = targets.len();
                self.targets = targets;
                self.sync_target_select();
                self.recorder_state = RecorderState::Idle;
                self.status = "Idle".to_string();
                self.push_log(format!("{count} capture targets loaded"));
                self.show_notification(format!("{count} capture targets loaded"));
            }
            AppEvent::TargetsLoaded(Err(message)) => {
                self.recorder_state = RecorderState::Failed;
                if is_permission_message(&message) {
                    self.permission_status = ScreenRecordingPermissionStatus::Missing;
                }
                self.show_error(message);
            }
            AppEvent::Started(Ok(job)) => {
                if !matches!(self.recorder_state, RecorderState::Starting) {
                    self.push_log(format!("ignored late start for job {}", job.id));
                    return;
                }
                self.active_job_id = Some(job.id);
                self.active_job_event_count = 0;
                self.apply_job_snapshot(job);
                self.start_job_poll();
                if self.quit_after_stop {
                    if let Some(job_id) = self.active_job_id {
                        self.submit_stop_job(job_id, "stopping recording before quit");
                    } else {
                        self.quit_after_stop = false;
                        self.show_error("No active daemon job to stop");
                    }
                    return;
                }
                self.show_notification("Recording submitted");
            }
            AppEvent::Started(Err(message)) => {
                if !matches!(self.recorder_state, RecorderState::Starting) {
                    self.push_log(format!("ignored late start failure: {message}"));
                    return;
                }
                self.set_app_window_capture_excluded(false);
                self.active_job_id = None;
                self.active_job_event_count = 0;
                self.active_output_path = None;
                self.quit_after_stop = false;
                self.recorder_state = RecorderState::Failed;
                if is_permission_message(&message) {
                    self.permission_status = ScreenRecordingPermissionStatus::Missing;
                }
                self.show_error(message);
            }
            AppEvent::JobPolled(Ok(job)) => {
                if self.should_accept_job(job.id) {
                    self.apply_job_snapshot(job);
                }
            }
            AppEvent::JobPolled(Err(message)) => {
                if self.active_job_id.is_some() {
                    self.set_app_window_capture_excluded(false);
                    self.recorder_state = RecorderState::Failed;
                    self.active_job_id = None;
                    self.active_output_path = None;
                    self.show_error(message);
                }
            }
            AppEvent::Paused(Ok(job)) => {
                if !matches!(self.recorder_state, RecorderState::Pausing) {
                    return;
                }
                self.apply_job_snapshot(job);
            }
            AppEvent::Paused(Err(message)) => {
                if matches!(self.recorder_state, RecorderState::Pausing) {
                    self.recorder_state = RecorderState::Recording;
                }
                self.show_error(message);
            }
            AppEvent::Resumed(Ok(job)) => {
                if !matches!(self.recorder_state, RecorderState::Resuming) {
                    return;
                }
                self.apply_job_snapshot(job);
            }
            AppEvent::Resumed(Err(message)) => {
                if matches!(self.recorder_state, RecorderState::Resuming) {
                    self.recorder_state = RecorderState::Paused;
                }
                self.show_error(message);
            }
            AppEvent::Stopped(Ok(job)) => {
                self.apply_job_snapshot(job);
            }
            AppEvent::Stopped(Err(message)) => {
                self.set_app_window_capture_excluded(false);
                self.active_job_id = None;
                self.active_job_event_count = 0;
                self.active_output_path = None;
                self.quit_after_stop = false;
                self.recorder_state = RecorderState::Failed;
                self.show_error(message);
            }
        }
    }

    fn apply_job_snapshot(&mut self, job: JobSnapshot) {
        if !self.should_accept_job(job.id) {
            return;
        }
        let quit_after_stop = self.quit_after_stop;
        let was_stopping = matches!(self.recorder_state, RecorderState::Stopping);
        self.active_job_id.get_or_insert(job.id);
        if let Some(path) = job.output_path.clone() {
            self.active_output_path = Some(path.clone());
            self.last_recording_dir = path.parent().map(Path::to_path_buf);
        }
        self.push_new_job_events(&job);
        if let Some(metrics) = job
            .events
            .iter()
            .rev()
            .find_map(|event| event.metrics.clone())
        {
            self.metrics = Some(metrics);
        }
        match job.status {
            JobStatus::Queued | JobStatus::Starting => {
                self.recorder_state = RecorderState::Starting;
                self.status = job
                    .target
                    .as_ref()
                    .map(|target| format!("Starting {}", target.name))
                    .unwrap_or_else(|| "Starting".to_string());
            }
            JobStatus::Recording => {
                self.recorder_state = RecorderState::Recording;
                self.status = job
                    .output_path
                    .as_ref()
                    .map(|path| format!("Recording to {}", path.display()))
                    .unwrap_or_else(|| "Recording".to_string());
            }
            JobStatus::Paused => {
                self.recorder_state = RecorderState::Paused;
                self.status = "Paused".to_string();
            }
            JobStatus::Finishing => {
                self.recorder_state = RecorderState::Stopping;
                self.status = "Stopping".to_string();
            }
            JobStatus::Completed => {
                self.set_app_window_capture_excluded(false);
                self.active_job_id = None;
                self.active_job_event_count = 0;
                self.active_output_path = None;
                self.recorder_state = RecorderState::Idle;
                self.status = job
                    .output_path
                    .as_ref()
                    .map(|path| format!("Saved to {}", path.display()))
                    .unwrap_or_else(|| "Recording completed".to_string());
                if quit_after_stop {
                    self.quit_after_stop = false;
                    self.push_log("recording saved; quitting Wrec");
                    // Signal quit via the egui context
                    // This is handled in the update() method
                } else if was_stopping {
                    self.open_last_recording_dir();
                    self.show_notification("Recording stopped");
                }
            }
            JobStatus::Failed | JobStatus::Cancelled => {
                self.set_app_window_capture_excluded(false);
                let message = latest_error_message(&job).unwrap_or_else(|| {
                    if matches!(job.status, JobStatus::Cancelled) {
                        "Recording cancelled".to_string()
                    } else {
                        "Recording failed".to_string()
                    }
                });
                self.active_job_id = None;
                self.active_job_event_count = 0;
                self.active_output_path = None;
                self.quit_after_stop = false;
                self.recorder_state = RecorderState::Failed;
                self.status = message.clone();
                if is_permission_message(&message) {
                    self.permission_status = ScreenRecordingPermissionStatus::Missing;
                }
                self.show_error(message);
            }
        }
    }

    fn push_new_job_events(&mut self, job: &JobSnapshot) {
        for event in job.events.iter().skip(self.active_job_event_count) {
            self.push_log(event.message.clone());
        }
        self.active_job_event_count = job.events.len();
    }

    fn should_accept_job(&self, job_id: u64) -> bool {
        self.active_job_id
            .map(|active_job_id| active_job_id == job_id)
            .unwrap_or(false)
    }

    fn start_job_poll(&self) {
        let Some(job_id) = self.active_job_id else {
            return;
        };
        let daemon = self.daemon.clone();
        let app_events = self.app_events.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            let result = daemon.show_job(job_id).map_err(agent_error_message);
            let stop = result.as_ref().map_or(true, |job| is_terminal_job(job));
            let _ = app_events.send(UiEvent::App(AppEvent::JobPolled(result)));
            if stop {
                break;
            }
        });
    }
}

impl eframe::App for WrecApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.process_events();

        // Store native window handle on first frame
        #[cfg(target_os = "windows")]
        {
            if self.native_window_handle == 0 {
                // FIXME: get native HWND from eframe for window capture exclusion
            }
        }

        // Handle quit after recording completes
        if self.quit_after_stop && self.active_job_id.is_none() {
            if matches!(self.recorder_state, RecorderState::Idle) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }

        // Render UI
        crate::ui::render(self, ctx, frame);

        // Repaint during recording for metrics updates
        if self.recorder_state.is_active_session() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }

        // Update notification timer
        self.update_notification(ctx.input(|i| i.unstable_dt) as f64);
    }
}

fn recording_params(target: CaptureTarget, settings: RecorderSettings) -> StartRecordingParams {
    StartRecordingParams {
        selector: Some(TargetSelector::Id {
            kind: target.kind,
            id: target.id,
        }),
        options: RecordingOptions {
            source_kind: Some(settings.source),
            fps: Some(settings.fps),
            codec: Some(settings.codec),
            quality: Some(settings.quality),
            resolution: Some(settings.resolution),
            output_dir: Some(settings.output_dir),
            include_cursor: Some(settings.include_cursor),
            include_system_audio: Some(settings.include_system_audio),
            hide_wrec: Some(settings.hide_wrec),
        },
        duration_ms: None,
        queue: false,
    }
}

fn agent_error_message(error: AgentError) -> String {
    format!("{} Next: {}", error.message, error.next)
}

fn is_terminal_job(job: &JobSnapshot) -> bool {
    matches!(
        job.status,
        JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
    )
}

fn latest_error_message(job: &JobSnapshot) -> Option<String> {
    job.events
        .iter()
        .rev()
        .find(|event| matches!(event.level, EventLevel::Error))
        .map(|event| event.message.clone())
}

fn metrics_label(metrics: &RecorderMetrics) -> String {
    format!(
        "{}s  {:.1} MB  {:.1} Mbps",
        metrics.elapsed_secs,
        metrics.output_bytes as f32 / 1_000_000.,
        metrics.estimated_bitrate_mbps
    )
}

fn zero_metrics_label() -> String {
    "0s  0.0 MB  0.0 Mbps".to_string()
}

fn is_permission_message(message: &str) -> bool {
    message.contains("Screen Recording") || message.contains("screen recording permission")
}

