use gpui::*;
use gpui_component::{
    button::{Button as UiButton, ButtonVariants as _, Toggle, ToggleGroup, ToggleVariants as _},
    input::{Input, InputEvent, InputState},
    notification::Notification,
    select::{Select, SelectEvent, SelectState},
    ActiveTheme as _, Icon as UiIcon, IconNamed, IndexPath, Root, Sizable as _, Theme, ThemeMode,
    WindowExt as _,
};
use gpui_platform::application;
use std::{
    borrow::Cow,
    collections::VecDeque,
    path::{Path, PathBuf},
    process::Command,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use wrec_core::{CaptureSourceKind, Codec, FrameRate, Quality, RecorderEngine, RecorderSettings};
use wrec_macos::{MacosRecorder, RecorderEvent};

type ControlSelect = SelectState<Vec<&'static str>>;

const MAX_LOGS: usize = 80;
const MAIN_CONTROL_WIDTH: f32 = 220.;
const SOURCE_OPTIONS: [&str; 2] = ["Display", "Window"];
const CODEC_OPTIONS: [&str; 2] = ["HEVC", "H.264"];
const QUALITY_OPTIONS: [&str; 3] = ["Balanced", "Efficient", "High"];

#[derive(Clone, Copy)]
enum PhosphorIcon {
    Cursor,
    FolderOpen,
    Record,
    Stop,
}

impl IconNamed for PhosphorIcon {
    fn path(self) -> SharedString {
        match self {
            Self::Cursor => "icons/phosphor/cursor.svg",
            Self::FolderOpen => "icons/phosphor/folder-open.svg",
            Self::Record => "icons/phosphor/record.svg",
            Self::Stop => "icons/phosphor/stop.svg",
        }
        .into()
    }
}

struct WrecAssets;

impl AssetSource for WrecAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let svg = match path {
            "icons/phosphor/cursor.svg" => phosphor_svgs::style::bold::CURSOR,
            "icons/phosphor/folder-open.svg" => phosphor_svgs::style::bold::FOLDER_OPEN,
            "icons/phosphor/record.svg" => phosphor_svgs::style::bold::RECORD,
            "icons/phosphor/stop.svg" => phosphor_svgs::style::bold::STOP,
            "icons/chevron-down.svg" => phosphor_svgs::style::bold::CARET_DOWN,
            "icons/circle-check.svg" => phosphor_svgs::style::bold::CHECK_CIRCLE,
            "icons/circle-x.svg" => phosphor_svgs::style::bold::X_CIRCLE,
            "icons/close.svg" => phosphor_svgs::style::bold::X,
            "icons/info.svg" => phosphor_svgs::style::bold::INFO,
            "icons/triangle-alert.svg" => phosphor_svgs::style::bold::WARNING,
            _ => return Ok(None),
        };

        Ok(Some(Cow::Borrowed(svg.as_bytes())))
    }

    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

struct WrecApp {
    engine: Arc<Mutex<MacosRecorder>>,
    settings: RecorderSettings,
    status: String,
    is_recording: bool,
    last_recording_dir: Option<PathBuf>,
    logs: VecDeque<String>,
    source_select: Entity<ControlSelect>,
    codec_select: Entity<ControlSelect>,
    quality_select: Entity<ControlSelect>,
    output_input: Entity<InputState>,
    _event_task: Task<()>,
}

impl WrecApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = RecorderSettings::default();

        let (events, receiver) = mpsc::channel();
        let event_task = cx.spawn_in(window, async move |this, cx| loop {
            while let Ok(event) = receiver.try_recv() {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.handle_event(event, window, cx);
                    })
                    .is_err()
                {
                    return;
                }
            }

            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
        });

        let source_select = cx.new(|cx| {
            SelectState::new(
                SOURCE_OPTIONS.to_vec(),
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let codec_select = cx.new(|cx| {
            SelectState::new(
                CODEC_OPTIONS.to_vec(),
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let quality_select = cx.new(|cx| {
            SelectState::new(
                QUALITY_OPTIONS.to_vec(),
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let output_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.output_dir.display().to_string())
                .placeholder("Output folder")
        });

        cx.subscribe_in(&source_select, window, Self::on_source_select)
            .detach();
        cx.subscribe_in(&codec_select, window, Self::on_codec_select)
            .detach();
        cx.subscribe_in(&quality_select, window, Self::on_quality_select)
            .detach();
        cx.subscribe_in(&output_input, window, Self::on_output_input)
            .detach();

        Self {
            engine: Arc::new(Mutex::new(MacosRecorder::new(events))),
            settings,
            status: "Idle".to_string(),
            is_recording: false,
            last_recording_dir: None,
            logs: VecDeque::new(),
            source_select,
            codec_select,
            quality_select,
            output_input,
            _event_task: event_task,
        }
    }

    fn on_source_select(
        &mut self,
        _: &Entity<ControlSelect>,
        event: &SelectEvent<Vec<&'static str>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        self.settings.source = match *value {
            "Window" => CaptureSourceKind::Window,
            _ => CaptureSourceKind::Display,
        };
        self.push_log(format!("source: {value}"));
        cx.notify();
    }

    fn on_codec_select(
        &mut self,
        _: &Entity<ControlSelect>,
        event: &SelectEvent<Vec<&'static str>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        self.settings.codec = match *value {
            "H.264" => Codec::H264,
            _ => Codec::Hevc,
        };
        self.push_log(format!("codec: {value}"));
        cx.notify();
    }

    fn on_quality_select(
        &mut self,
        _: &Entity<ControlSelect>,
        event: &SelectEvent<Vec<&'static str>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        self.settings.quality = match *value {
            "Efficient" => Quality::Efficient,
            "High" => Quality::High,
            _ => Quality::Balanced,
        };
        self.push_log(format!("quality: {value}"));
        cx.notify();
    }

    fn on_output_input(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
            let value = input.read(cx).value().trim().to_string();
            if !value.is_empty() {
                self.settings.output_dir = PathBuf::from(value);
                self.push_log(format!(
                    "output dir: {}",
                    self.settings.output_dir.display()
                ));
                cx.notify();
            }
        }
    }

    fn set_fps(&mut self, fps: FrameRate, cx: &mut Context<Self>) {
        self.settings.fps = fps;
        self.push_log(format!("fps: {}", fps.as_u32()));
        cx.notify();
    }

    fn choose_output_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = choose_output_dir() else {
            self.push_log("output picker cancelled");
            return;
        };

        self.settings.output_dir = path.clone();
        let value = path.display().to_string();
        self.output_input.update(cx, |input, cx| {
            input.set_value(value.clone(), window, cx);
        });
        self.push_log(format!("output dir: {value}"));
        cx.notify();
    }

    fn toggle_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_recording {
            let result = self.engine.lock().unwrap().stop();
            match result {
                Ok(()) => {
                    self.is_recording = false;
                    self.status = "Stopped".to_string();
                    self.push_log("Stopped");
                    if let Some(path) = self.last_recording_dir.as_deref() {
                        match open_path(path) {
                            Ok(()) => self.push_log(format!("opened: {}", path.display())),
                            Err(err) => {
                                self.push_log(format!("open failed: {err}"));
                                window.push_notification(
                                    Notification::error(format!(
                                        "Could not open output folder: {err}"
                                    )),
                                    cx,
                                );
                            }
                        }
                    }
                    window.push_notification(Notification::success("Recording stopped"), cx);
                }
                Err(err) => self.show_error(err.to_string(), window, cx),
            }
            return;
        }

        self.push_log("listing capture targets");
        let result = self.engine.lock().unwrap().list_targets();
        let target = match result {
            Ok(targets) => targets
                .iter()
                .find(|target| target.kind == self.settings.source)
                .cloned()
                .or_else(|| targets.into_iter().next()),
            Err(err) => {
                self.show_error(err.to_string(), window, cx);
                return;
            }
        };

        let Some(target) = target else {
            self.show_error("No capture target found", window, cx);
            return;
        };

        self.push_log(format!("target: {}", target.name));
        let result = self
            .engine
            .lock()
            .unwrap()
            .start(target, self.settings.clone());
        match result {
            Ok(session) => {
                self.is_recording = true;
                self.status = format!("Recording to {}", session.output_path.display());
                self.last_recording_dir = session.output_path.parent().map(Path::to_path_buf);
                self.push_log(self.status.clone());
                window.push_notification(Notification::success("Recording started"), cx);
            }
            Err(err) => self.show_error(err.to_string(), window, cx),
        }
    }

    fn handle_event(&mut self, event: RecorderEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            RecorderEvent::Log(message) => {
                if message.contains("recording started") {
                    self.status = "Recording".to_string();
                }
                self.push_log(message);
            }
            RecorderEvent::Failed(message) => {
                self.is_recording = false;
                self.show_error(message, window, cx);
            }
            RecorderEvent::Exited { success, status } => {
                self.is_recording = false;
                if success {
                    self.push_log(format!("helper exited: {status}"));
                } else {
                    self.show_error(format!("helper exited: {status}"), window, cx);
                }
            }
        }
    }

    fn show_error(
        &mut self,
        message: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = message.into();
        self.status = message.clone();
        self.push_log(format!("error: {message}"));
        tracing::error!("{message}");
        window.push_notification(Notification::error(message).autohide(false), cx);
    }

    fn push_log(&mut self, message: impl Into<String>) {
        let message = message.into();
        tracing::info!("{message}");
        self.logs.push_back(message);
        while self.logs.len() > MAX_LOGS {
            self.logs.pop_front();
        }
    }
}

impl Render for WrecApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let foreground = cx.theme().foreground;
        let muted_foreground = cx.theme().muted_foreground;
        let notification_layer = Root::render_notification_layer(window, cx);
        let (record_icon, record_label, record_tip, record_is_idle) = if self.is_recording {
            (PhosphorIcon::Stop, "Stop", "Stop recording", false)
        } else {
            (PhosphorIcon::Record, "Rec", "Start recording", true)
        };
        let border = hsla(0., 0., 1., 0.12);
        let panel = hsla(0., 0., 0.05, 0.74);

        div()
            .id("wrec-root")
            .relative()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(border)
            .bg(panel)
            .text_color(foreground)
            .text_lg()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .gap_3()
                    .pb_3()
                    .px_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(px(38.))
                            .window_control_area(WindowControlArea::Drag)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(window_dot(
                                        "window-close",
                                        hsla(0., 0.82, 0.62, 1.),
                                        hsla(0., 0.82, 0.68, 1.),
                                        WindowControlArea::Close,
                                        |_, window, _| window.remove_window(),
                                    ))
                                    .child(window_dot(
                                        "window-minimize",
                                        hsla(0.12, 0.88, 0.58, 1.),
                                        hsla(0.12, 0.88, 0.66, 1.),
                                        WindowControlArea::Min,
                                        |_, window, _| window.minimize_window(),
                                    ))
                                    .child(window_dot(
                                        "window-maximize",
                                        hsla(0.37, 0.62, 0.5, 1.),
                                        hsla(0.37, 0.62, 0.58, 1.),
                                        WindowControlArea::Max,
                                        |_, window, _| window.zoom_window(),
                                    )),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(foreground)
                                    .child("wrec"),
                            )
                            .child(div().w(px(58.))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(MAIN_CONTROL_WIDTH))
                                    .h(px(44.))
                                    .child(
                                        Select::new(&self.source_select)
                                            .large()
                                            .h(px(44.))
                                            .placeholder("Record")
                                            .title_prefix("Record: ")
                                            .menu_width(px(MAIN_CONTROL_WIDTH))
                                            .menu_max_h(rems(7.)),
                                    ),
                            )
                            .child(fps_toggle(self.settings.fps, cx))
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(MAIN_CONTROL_WIDTH))
                                    .h(px(44.))
                                    .child(
                                        Select::new(&self.codec_select)
                                            .large()
                                            .h(px(44.))
                                            .placeholder("Format")
                                            .title_prefix("Format: ")
                                            .menu_width(px(MAIN_CONTROL_WIDTH)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(record_button(
                                record_icon,
                                record_label,
                                record_tip,
                                record_is_idle,
                                cx,
                            ))
                            .child(
                                Toggle::new("cursor-toggle")
                                    .outline()
                                    .large()
                                    .w(px(MAIN_CONTROL_WIDTH))
                                    .h(px(44.))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                UiIcon::new(PhosphorIcon::Cursor)
                                                    .large()
                                                    .text_color(muted_foreground),
                                            )
                                            .child("Cursor"),
                                    )
                                    .checked(self.settings.include_cursor)
                                    .tooltip("Capture cursor")
                                    .on_click(cx.listener(|this, checked, _, cx| {
                                        this.settings.include_cursor = *checked;
                                        this.push_log(format!(
                                            "cursor: {}",
                                            if *checked { "on" } else { "off" }
                                        ));
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(MAIN_CONTROL_WIDTH))
                                    .h(px(44.))
                                    .child(
                                        Select::new(&self.quality_select)
                                            .large()
                                            .h(px(44.))
                                            .placeholder("Quality")
                                            .title_prefix("Quality: ")
                                            .menu_width(px(MAIN_CONTROL_WIDTH)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .h(px(44.))
                                    .child(Input::new(&self.output_input).large().h(px(44.))),
                            )
                            .child(
                                UiButton::new("choose-output-dir")
                                    .large()
                                    .outline()
                                    .w(px(MAIN_CONTROL_WIDTH))
                                    .h(px(44.))
                                    .icon(
                                        UiIcon::new(PhosphorIcon::FolderOpen)
                                            .large()
                                            .text_color(muted_foreground),
                                    )
                                    .label("Folder")
                                    .tooltip("Choose output folder")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose_output_dir(window, cx);
                                    })),
                            ),
                    ),
            )
            .children(notification_layer)
    }
}

fn record_button(
    icon: PhosphorIcon,
    label: &'static str,
    tooltip: &'static str,
    is_idle: bool,
    cx: &mut Context<WrecApp>,
) -> UiButton {
    let theme = cx.theme();
    let button = UiButton::new("record-button")
        .large()
        .w(px(MAIN_CONTROL_WIDTH))
        .h(px(44.))
        .icon(UiIcon::new(icon).large().text_color(if is_idle {
            theme.button_primary_foreground
        } else {
            theme.danger_foreground
        }))
        .label(label)
        .tooltip(tooltip)
        .on_click(cx.listener(|this, _, window, cx| {
            this.toggle_recording(window, cx);
            cx.notify();
        }));

    if is_idle {
        button.primary()
    } else {
        button.danger()
    }
}

fn fps_toggle(fps: FrameRate, cx: &mut Context<WrecApp>) -> impl IntoElement {
    ToggleGroup::new("fps-toggle")
        .segmented()
        .outline()
        .large()
        .w(px(MAIN_CONTROL_WIDTH))
        .h(px(44.))
        .child(
            Toggle::new("fps-30")
                .label("30")
                .checked(matches!(fps, FrameRate::Fps30))
                .w(px(MAIN_CONTROL_WIDTH / 2.))
                .h(px(44.)),
        )
        .child(
            Toggle::new("fps-60")
                .label("60")
                .checked(matches!(fps, FrameRate::Fps60))
                .w(px(MAIN_CONTROL_WIDTH / 2.))
                .h(px(44.)),
        )
        .on_click(cx.listener(|this, checks: &Vec<bool>, _, cx| {
            let fps = if checks.get(1).copied().unwrap_or(false) {
                FrameRate::Fps60
            } else {
                FrameRate::Fps30
            };
            this.set_fps(fps, cx);
        }))
}

fn window_dot(
    id: &'static str,
    color: Hsla,
    hover_color: Hsla,
    area: WindowControlArea,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .size(px(14.))
        .rounded_full()
        .bg(color)
        .border_1()
        .border_color(hsla(0., 0., 0., 0.18))
        .hover(move |this| this.bg(hover_color))
        .active(move |this| this.bg(color.opacity(0.72)))
        .window_control_area(area)
        .on_click(on_click)
}

fn choose_output_dir() -> Option<PathBuf> {
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

fn open_path(path: &Path) -> std::io::Result<()> {
    Command::new("open").arg(path).spawn().map(|_| ())
}

fn main() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    application().with_assets(WrecAssets).run(|cx: &mut App| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.activate(true);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(740.), px(206.)), cx)),
            window_min_size: Some(size(px(680.), px(198.))),
            titlebar: None,
            window_background: WindowBackgroundAppearance::Blurred,
            window_decorations: Some(WindowDecorations::Client),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                window.activate_window();
                window.set_window_title("wrec");
                let app = cx.new(|cx| WrecApp::new(window, cx));
                cx.new(|cx| Root::new(app, window, cx))
            })
            .expect("open window");
        })
        .detach();
    });
}
