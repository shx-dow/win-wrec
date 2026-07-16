use crate::{
    app::WrecApp,
    assets::{PhosphorIcon, GEIST_FONT_FAMILY, GEIST_MONO_FONT_FAMILY},
    platform::CliInstallStatus,
};
use domain::{
    CaptureSourceKind, CaptureTarget, FrameRate, Quality, RecorderMetrics, Resolution,
    ScreenRecordingPermissionStatus,
};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::{Button as UiButton, ButtonVariants as _},
    input::Input,
    label::Label,
    notification::Notification,
    select::{Select, SelectItem, SelectState},
    switch::Switch,
    ActiveTheme as _, Colorize as _, Disableable as _, Icon as UiIcon, Root, Sizable as _, Theme,
    ThemeMode, WindowExt as _,
};
pub(crate) type ControlSelect = SelectState<Vec<&'static str>>;
pub(crate) type LimitedSelect = SelectState<Vec<LimitedOption>>;
pub(crate) type TargetSelect = SelectState<Vec<TargetOption>>;

pub(crate) const CONTROL_HEIGHT: f32 = 30.;
const RECORD_BUTTON_HEIGHT: f32 = 48.;
pub(crate) const WINDOW_WIDTH: f32 = 680.;
pub(crate) const WINDOW_HEIGHT: f32 = 580.;
pub(crate) const WINDOW_MIN_WIDTH: f32 = 640.;
pub(crate) const WINDOW_MIN_HEIGHT: f32 = 540.;
pub(crate) const SOURCE_OPTIONS: [&str; 2] = ["Display", "Window"];
pub(crate) const CODEC_OPTIONS: [&str; 2] = ["HEVC", "H.264"];
pub(crate) const QUALITY_OPTIONS: [&str; 3] = ["Balanced", "Efficient", "High"];

const SIDEBAR_WIDTH: f32 = 140.;
const TITLE_BAR_HEIGHT: f32 = 40.;
const NATIVE_WINDOW_CONTROLS_WIDTH: f32 = 72.;
const NOTIFICATION_WIDTH: f32 = 320.;

#[derive(Clone, Copy)]
struct ThemePalette {
    background: u32,
    foreground: u32,
    card: u32,
    card_foreground: u32,
    popover: u32,
    popover_foreground: u32,
    primary: u32,
    primary_hover: u32,
    primary_active: u32,
    primary_foreground: u32,
    secondary: u32,
    secondary_hover: u32,
    secondary_active: u32,
    secondary_foreground: u32,
    muted: u32,
    muted_foreground: u32,
    accent: u32,
    accent_foreground: u32,
    destructive: u32,
    destructive_foreground: u32,
    border: u32,
    input: u32,
    chart_1: u32,
    chart_2: u32,
    chart_3: u32,
    chart_4: u32,
    chart_5: u32,
    sidebar: u32,
    sidebar_foreground: u32,
    sidebar_primary: u32,
    sidebar_primary_foreground: u32,
    sidebar_accent: u32,
    sidebar_accent_foreground: u32,
    sidebar_border: u32,
}

const LIGHT_PALETTE: ThemePalette = ThemePalette {
    background: 0xf5f5f4,
    foreground: 0x0c0c0d,
    card: 0xffffff,
    card_foreground: 0x0c0c0d,
    popover: 0xffffff,
    popover_foreground: 0x0c0c0d,
    primary: 0x0c0c0d,
    primary_hover: 0x27272a,
    primary_active: 0x18181b,
    primary_foreground: 0xffffff,
    secondary: 0xe4e4e7,
    secondary_hover: 0xd4d4d8,
    secondary_active: 0xa1a1aa,
    secondary_foreground: 0x0c0c0d,
    muted: 0xecedee,
    muted_foreground: 0x78787e,
    accent: 0xe8e8ea,
    accent_foreground: 0x0c0c0d,
    destructive: 0xdc2626,
    destructive_foreground: 0xffffff,
    border: 0xd4d4d8,
    input: 0xd4d4d8,
    chart_1: 0x0c0c0d,
    chart_2: 0xe4e4e7,
    chart_3: 0xecedee,
    chart_4: 0xc8c8ce,
    chart_5: 0x0c0c0d,
    sidebar: 0xf0efed,
    sidebar_foreground: 0x78787e,
    sidebar_primary: 0x0c0c0d,
    sidebar_primary_foreground: 0xffffff,
    sidebar_accent: 0xe4e3e0,
    sidebar_accent_foreground: 0x0c0c0d,
    sidebar_border: 0xdcdbd7,
};

const DARK_PALETTE: ThemePalette = ThemePalette {
    background: 0x080808,
    foreground: 0xf1f1f3,
    card: 0x101012,
    card_foreground: 0xf1f1f3,
    popover: 0x141416,
    popover_foreground: 0xf1f1f3,
    primary: 0xf1f1f3,
    primary_hover: 0xc8c8ce,
    primary_active: 0xa8a8b0,
    primary_foreground: 0x080808,
    secondary: 0x1a1a1e,
    secondary_hover: 0x26262a,
    secondary_active: 0x2f2f33,
    secondary_foreground: 0xf1f1f3,
    muted: 0x121214,
    muted_foreground: 0x9999a0,
    accent: 0x1a1a1e,
    accent_foreground: 0xf1f1f3,
    destructive: 0xe5484d,
    destructive_foreground: 0xffffff,
    border: 0x1e1e22,
    input: 0x1e1e22,
    chart_1: 0xf1f1f3,
    chart_2: 0x1a1a1e,
    chart_3: 0x1a1a1e,
    chart_4: 0x2f2f33,
    chart_5: 0xf1f1f3,
    sidebar: 0x0a0a0b,
    sidebar_foreground: 0x787880,
    sidebar_primary: 0xf1f1f3,
    sidebar_primary_foreground: 0x080808,
    sidebar_accent: 0x18181c,
    sidebar_accent_foreground: 0xf1f1f3,
    sidebar_border: 0x16161a,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppTab {
    General,
    Settings,
    Cli,
    About,
    Nerd,
}

impl AppTab {
    fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Settings => "settings",
            Self::Cli => "cli",
            Self::About => "about",
            Self::Nerd => "nerd",
        }
    }

    fn is_active(self, active_tab: Self) -> bool {
        self == active_tab
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TargetOption {
    key: SharedString,
    title: SharedString,
}

impl TargetOption {
    pub(crate) fn new(target: &CaptureTarget) -> Self {
        Self {
            key: target_key(target).into(),
            title: target.name.clone().into(),
        }
    }

    pub(crate) fn key(&self) -> &SharedString {
        &self.key
    }
}

impl SelectItem for TargetOption {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.key
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LimitedOption {
    value: SharedString,
    title: SharedString,
    disabled: bool,
}

impl LimitedOption {
    fn new(label: &'static str, disabled: bool) -> Self {
        Self {
            value: label.into(),
            title: label.into(),
            disabled,
        }
    }
}

impl SelectItem for LimitedOption {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }

    fn disabled(&self) -> bool {
        self.disabled
    }
}

pub(crate) fn resolution_options_for(quality: Quality) -> Vec<LimitedOption> {
    [
        (Resolution::Native, "Original"),
        (Resolution::R4k, "4K"),
        (Resolution::R2k, "2K"),
        (Resolution::R1080p, "1080p"),
        (Resolution::R720p, "720p"),
    ]
    .into_iter()
    .map(|(resolution, label)| LimitedOption::new(label, resolution_disabled(quality, resolution)))
    .collect()
}

pub(crate) fn fps_options_for(quality: Quality) -> Vec<LimitedOption> {
    [(FrameRate::Fps30, "30 FPS"), (FrameRate::Fps60, "60 FPS")]
        .into_iter()
        .map(|(fps, label)| LimitedOption::new(label, fps_disabled(quality, fps)))
        .collect()
}

pub(crate) fn resolution_disabled(quality: Quality, resolution: Resolution) -> bool {
    quality
        .max_resolution()
        .is_some_and(|cap| resolution.capped_at(cap) != resolution)
}

pub(crate) fn fps_disabled(quality: Quality, fps: FrameRate) -> bool {
    fps.capped_at(quality.max_fps()) != fps
}

impl WrecApp {
    pub(crate) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_dark = cx.theme().mode.is_dark();
        div()
            .id("wrec-titlebar")
            .flex()
            .items_center()
            .justify_between()
            .h(px(TITLE_BAR_HEIGHT))
            .flex_shrink_0()
            .pl(px(16.))
            .pr_2p5()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .w(px(NATIVE_WINDOW_CONTROLS_WIDTH))
                    .h_full()
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(theme_toggle(is_dark, cx))
    }

    pub(crate) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active_tab;

        let general = Self::sidebar_item(
            AppTab::General,
            PhosphorIcon::Gauge,
            "General",
            AppTab::General.is_active(active),
            cx,
        );
        let cli = Self::sidebar_item(
            AppTab::Cli,
            PhosphorIcon::Terminal,
            "CLI",
            AppTab::Cli.is_active(active),
            cx,
        );
        let nerd = self.show_nerd_logs.then(|| {
            Self::sidebar_item(
                AppTab::Nerd,
                PhosphorIcon::Pulse,
                "Nerd",
                AppTab::Nerd.is_active(active),
                cx,
            )
        });
        let settings = Self::sidebar_item(
            AppTab::Settings,
            PhosphorIcon::Gear,
            "Settings",
            AppTab::Settings.is_active(active),
            cx,
        );
        let about = Self::sidebar_item(
            AppTab::About,
            PhosphorIcon::Info,
            "About",
            AppTab::About.is_active(active),
            cx,
        );

        div()
            .id("wrec-sidebar")
            .flex()
            .flex_col()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                div()
                    .id("wrec-sidebar-nav")
                    .flex()
                    .flex_col()
                    .w_full()
                    .h_full()
                    .pt_4()
                    .child(
                        div().flex().flex_col().px_2().pb_4().child(
                            div()
                                .text_xs()
                                .text_color(switch_on_color(cx))
                                .font_weight(FontWeight::BOLD)
                                .child("wrec"),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .gap_0p5()
                            .child(general)
                            .child(cli)
                            .when_some(nerd, |this, item| this.child(item))
                            .child(
                                div().flex_1().min_h(px(0.)),
                            )
                            .child(settings)
                            .child(about),
                    ),
            )
    }

    fn sidebar_item(
        tab: AppTab,
        icon: PhosphorIcon,
        label: &'static str,
        active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hover_bg = cx.theme().sidebar_accent;
        let accent = crate::ui::switch_on_color(cx);

        div()
            .id(format!("sidebar-nav-{}", tab.id()))
            .flex()
            .items_center()
            .w_full()
            .pl_1()
            .pr_2()
            .child(
                div()
                    .flex_none()
                    .w(px(3.0))
                    .h(px(16.0))
                    .rounded_full()
                    .when(active, |this| this.bg(accent)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .flex_1()
                    .min_w(px(0.))
                    .h(px(CONTROL_HEIGHT))
                    .px_2p5()
                    .rounded_lg()
                    .text_base()
                    .font_weight(if active {
                        FontWeight::BOLD
                    } else {
                        FontWeight::SEMIBOLD
                    })
                    .cursor_pointer()
                    .when(active, |this| {
                        this.bg(hover_bg)
                            .text_color(cx.theme().sidebar_accent_foreground)
                    })
                    .when(!active, |this| {
                        this.text_color(cx.theme().sidebar_foreground)
                            .hover(|this| {
                                this.bg(hover_bg)
                                    .text_color(cx.theme().sidebar_accent_foreground)
                            })
                    })
                    .child(
                        UiIcon::new(icon)
                            .size(px(16.))
                            .flex_shrink_0(),
                    )
                    .child(div().flex_1().min_w(px(0.)).truncate().child(label)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.active_tab != tab {
                    this.active_tab = tab;
                    cx.notify();
                }
            }))
    }

    pub(crate) fn render_general_tab(
        &self,
        record_icon: PhosphorIcon,
        record_label: &'static str,
        record_tip: &'static str,
        record_is_idle: bool,
        record_disabled: bool,
        show_pause_button: bool,
        pause_icon: PhosphorIcon,
        pause_label: &'static str,
        pause_tip: &'static str,
        pause_disabled: bool,
        controls_disabled: bool,
        muted_foreground: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let control = |label: &'static str| {
            div()
                .text_xs()
                .text_color(muted_foreground)
                .font_weight(FontWeight::SEMIBOLD)
                .mb_1()
                .child(label)
        };

        let (status_label, status_color) = if self.recorder_state.is_recording() {
            ("Recording", cx.theme().danger)
        } else if self.recorder_state.is_paused() {
            ("Paused", cx.theme().foreground.opacity(0.5))
        } else {
            ("Ready", Hsla::from(rgb(0x22c55e)))
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(cx.theme().foreground)
                                    .child("Capture"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .w(px(6.0))
                                            .h(px(6.0))
                                            .rounded_full()
                                            .bg(status_color),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted_foreground)
                                            .child(status_label),
                                    ),
                            ),
                    )
                    .child(div().h(px(1.0)).w_full().bg(cx.theme().border)),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .flex_1()
                            .min_w(px(0.))
                            .child(control("Source"))
                            .child(
                                div().h(px(CONTROL_HEIGHT)).child(
                                    Select::new(&self.source_select)
                                        .large()
                                        .h(px(CONTROL_HEIGHT))
                                        .placeholder("Source")
                                        .menu_max_h(rems(7.))
                                        .disabled(controls_disabled),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .flex_1()
                            .min_w(px(0.))
                            .child(control("Target"))
                            .child(
                                div().h(px(CONTROL_HEIGHT)).child(
                                    Select::new(&self.target_select)
                                        .large()
                                        .h(px(CONTROL_HEIGHT))
                                        .placeholder("Target")
                                        .search_placeholder("Search targets")
                                        .menu_max_h(rems(14.))
                                        .disabled(controls_disabled),
                                ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .flex_1()
                            .min_w(px(0.))
                            .child(control("Format"))
                            .child(
                                div().h(px(CONTROL_HEIGHT)).child(
                                    Select::new(&self.codec_select)
                                        .large()
                                        .h(px(CONTROL_HEIGHT))
                                        .placeholder("Format")
                                        .disabled(controls_disabled),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .flex_1()
                            .min_w(px(0.))
                            .child(control("Preset"))
                            .child(
                                div().h(px(CONTROL_HEIGHT)).child(
                                    Select::new(&self.quality_select)
                                        .large()
                                        .h(px(CONTROL_HEIGHT))
                                        .placeholder("Preset")
                                        .disabled(controls_disabled),
                                ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .flex_1()
                            .min_w(px(0.))
                            .child(control("Resolution"))
                            .child(
                                div().h(px(CONTROL_HEIGHT)).child(
                                    Select::new(&self.resolution_select)
                                        .large()
                                        .h(px(CONTROL_HEIGHT))
                                        .placeholder("Resolution")
                                        .disabled(controls_disabled),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .flex_1()
                            .min_w(px(0.))
                            .child(control("FPS"))
                            .child(
                                div().h(px(CONTROL_HEIGHT)).child(
                                    Select::new(&self.fps_select)
                                        .large()
                                        .h(px(CONTROL_HEIGHT))
                                        .placeholder("Frame Rate")
                                        .disabled(controls_disabled),
                                ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_6()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Label::new("Cursor")
                                    .text_sm()
                                    .text_color(muted_foreground),
                            )
                            .child(
                                Switch::new("cursor-switch")
                                    .checked(self.settings.include_cursor)
                                    .color(switch_on_color(cx))
                                    .tooltip("Capture cursor")
                                    .disabled(controls_disabled)
                                    .on_click(cx.listener(|this, checked, _, cx| {
                                        this.set_include_cursor(*checked, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Label::new("System Audio")
                                    .text_sm()
                                    .text_color(muted_foreground),
                            )
                            .child(
                                Switch::new("system-audio-switch")
                                    .checked(self.settings.include_system_audio)
                                    .color(switch_on_color(cx))
                                    .tooltip("Capture system audio")
                                    .disabled(controls_disabled)
                                    .on_click(cx.listener(|this, checked, _, cx| {
                                        this.set_include_system_audio(*checked, cx);
                                    })),
                            ),
                    ),
            )
            .child(if show_pause_button {
                div()
                    .flex()
                    .gap_2()
                    .child(
                        pause_button(pause_icon, pause_label, pause_tip, pause_disabled, cx)
                            .flex_1(),
                    )
                    .child(
                        record_button(
                            record_icon,
                            record_label,
                            record_tip,
                            record_is_idle,
                            record_disabled,
                            cx,
                        )
                        .flex_1(),
                    )
                    .into_any_element()
            } else {
                record_button(
                    record_icon,
                    record_label,
                    record_tip,
                    record_is_idle,
                    record_disabled,
                    cx,
                )
                .w_full()
                .rounded_lg()
                .into_any_element()
            })
    }

    pub(crate) fn render_settings_tab(
        &self,
        controls_disabled: bool,
        muted_foreground: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .gap_3()
            .child(card_section(vec![
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .min_h(px(CONTROL_HEIGHT))
                    .child(row_label("Screen Recording"))
                    .child(permission_state_button(
                        self.permission_status,
                        self.permission_busy,
                        cx,
                    )),
                switch_row(
                    "Hide wrec",
                    if self.settings.hide_wrec { "On" } else { "Off" },
                    muted_foreground,
                    Switch::new("hide-window-switch")
                        .checked(self.settings.hide_wrec)
                        .color(switch_on_color(cx))
                        .tooltip("Hide wrec from recording")
                        .disabled(controls_disabled)
                        .on_click(cx.listener(|this, checked, _, cx| {
                            this.set_hide_wrec(*checked, cx);
                        })),
                ),
                switch_row(
                    "Logs",
                    if self.show_nerd_logs { "On" } else { "Off" },
                    muted_foreground,
                    Switch::new("logs-switch")
                        .checked(self.show_nerd_logs)
                        .color(switch_on_color(cx))
                        .tooltip("Show Nerd tab")
                        .on_click(cx.listener(|this, checked, _, cx| {
                            this.set_show_nerd_logs(*checked, cx);
                        })),
                ),
            ], cx))
            .child(card_section(vec![
                section_label("Output", muted_foreground),
                div().w_full().h(px(CONTROL_HEIGHT)).child(
                    Input::new(&self.output_input)
                        .large()
                        .h(px(CONTROL_HEIGHT))
                        .disabled(controls_disabled),
                ),
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        UiButton::new("choose-output-dir")
                            .secondary()
                            .flex_1()
                            .h(px(CONTROL_HEIGHT))
                            .font_weight(FontWeight::SEMIBOLD)
                            .icon(
                                UiIcon::new(PhosphorIcon::FolderOpen).text_color(muted_foreground),
                            )
                            .label("Choose")
                            .tooltip("Choose output folder")
                            .disabled(controls_disabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_output_dir(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        UiButton::new("open-last-recording-dir")
                            .secondary()
                            .flex_1()
                            .h(px(CONTROL_HEIGHT))
                            .font_weight(FontWeight::SEMIBOLD)
                            .icon(
                                UiIcon::new(PhosphorIcon::FolderOpen).text_color(muted_foreground),
                            )
                            .label("Open")
                            .tooltip("Open last recording folder")
                            .disabled(self.last_recording_dir.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_last_recording_dir(window, cx);
                            })),
                    ),
            ], cx))
    }

    pub(crate) fn render_cli_tab(
        &self,
        muted_foreground: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let cli_command = crate::platform::cli_install_command();
        let cli_status_color = match self.cli_install_status {
            CliInstallStatus::Conflict => cx.theme().danger,
            _ => muted_foreground,
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .gap_3()
            .child(card_section(vec![
                section_label("Status", muted_foreground),
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .min_h(px(CONTROL_HEIGHT))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_baseline()
                            .gap_2()
                            .min_w(px(0.))
                            .child(row_label("Status"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cli_status_color)
                                    .truncate()
                                    .child(self.cli_install_status.label()),
                            ),
                    )
                    .child(
                        UiButton::new("cli-refresh-install")
                            .ghost()
                            .compact()
                            .size(px(CONTROL_HEIGHT))
                            .icon(UiIcon::new(PhosphorIcon::Refresh).text_color(muted_foreground))
                            .tooltip("Refresh CLI install status")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_cli_install_status(cx);
                            })),
                    ),
            ], cx))
            .child(card_section(vec![
                section_label("Install", muted_foreground),
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .min_h(px(CONTROL_HEIGHT))
                    .child(row_label("Install command"))
                    .child(
                        UiButton::new("cli-copy-install")
                            .secondary()
                            .compact()
                            .h(px(CONTROL_HEIGHT))
                            .font_weight(FontWeight::SEMIBOLD)
                            .icon(UiIcon::new(PhosphorIcon::Clipboard).text_color(muted_foreground))
                            .label("Copy")
                            .tooltip("Copy CLI install command")
                            .disabled(cli_command.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.copy_cli_install_command(window, cx);
                            })),
                    ),
            ], cx))
    }

    pub(crate) fn render_nerds_tab(
        &self,
        metrics_label: Option<String>,
        muted_foreground: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let metrics_label = metrics_label.unwrap_or_else(zero_metrics_label);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .gap_3()
            .child(card_section(vec![
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .min_h(px(CONTROL_HEIGHT))
                    .child(row_label("Logs"))
                    .child(
                        UiButton::new("open-recordings-data-dir")
                            .secondary()
                            .compact()
                            .h(px(CONTROL_HEIGHT))
                            .font_weight(FontWeight::SEMIBOLD)
                            .icon(
                                UiIcon::new(PhosphorIcon::FolderOpen).text_color(muted_foreground),
                            )
                            .label("Open")
                            .tooltip("Open recordings data folder")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_recordings_data_dir(window, cx);
                            })),
                    ),
            ], cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .px_3()
                    .child(
                        div()
                            .max_w_full()
                            .truncate()
                            .text_center()
                            .text_size(px(28.))
                            .line_height(relative(1.2))
                            .font_weight(FontWeight::SEMIBOLD)
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_color(cx.theme().foreground)
                            .child(metrics_label),
                    ),
            )
    }

    pub(crate) fn render_about_tab(
        &self,
        muted_foreground: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .gap_3()
            .child(card_section(vec![
                section_label("About", muted_foreground),
                plain_info_row(
                    "Version",
                    env!("CARGO_PKG_VERSION"),
                    muted_foreground,
                ),
                div().child(
                    UiButton::new("open-github")
                        .secondary()
                        .w_full()
                        .h(px(CONTROL_HEIGHT))
                        .font_weight(FontWeight::SEMIBOLD)
                        .icon(UiIcon::new(PhosphorIcon::Github).text_color(muted_foreground))
                        .label("GitHub")
                        .tooltip("Open GitHub repository")
                        .on_click(cx.listener(|this, _, window, cx| {
                            match crate::platform::open_url(crate::app::GITHUB_URL) {
                                Ok(()) => this.push_log("opened GitHub repository"),
                                Err(err) => {
                                    this.push_log(format!("open GitHub failed: {err}"));
                                    push_app_notification(
                                        window,
                                        Notification::new().message(format!(
                                            "Could not open GitHub repository: {err}"
                                        )),
                                        cx,
                                    );
                                }
                            }
                            cx.notify();
                        })),
                ),
            ], cx))
    }
}

impl Render for WrecApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let foreground = cx.theme().foreground;
        let muted_foreground = cx.theme().muted_foreground;
        let background = cx.theme().background;
        let border = cx.theme().border;
        let notification_layer = Root::render_notification_layer(window, cx);
        let active_session = self.recorder_state.is_active_session();
        let (record_icon, record_label, record_tip, record_is_idle) = if active_session {
            (PhosphorIcon::Stop, "Stop", "Stop recording", false)
        } else {
            (PhosphorIcon::FilmReel, "Record", "Start recording", true)
        };
        let (pause_icon, pause_label, pause_tip) = if self.recorder_state.is_paused() {
            (PhosphorIcon::Play, "Resume", "Resume recording")
        } else {
            (PhosphorIcon::Pause, "Pause", "Pause recording")
        };
        let record_disabled = matches!(
            self.recorder_state,
            crate::app::RecorderState::Starting
                | crate::app::RecorderState::Pausing
                | crate::app::RecorderState::Resuming
                | crate::app::RecorderState::Stopping
        ) || (!active_session
            && (self.permission_busy || !self.permission_status.is_granted()));
        let pause_disabled = matches!(
            self.recorder_state,
            crate::app::RecorderState::Pausing
                | crate::app::RecorderState::Resuming
                | crate::app::RecorderState::Stopping
        );
        let controls_disabled =
            self.recorder_state.is_busy() || self.permission_busy || active_session;
        let metrics_label = Some(if active_session || self.recorder_state.is_recording() {
            self.cached_metrics_label.clone()
        } else {
            zero_metrics_label()
        });

        div()
            .id("wrec-root")
            .on_action(cx.listener(WrecApp::on_minimize_action))
            .on_action(cx.listener(WrecApp::on_quit_action))
            .relative()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(border)
            .bg(background)
            .text_color(foreground)
            .text_size(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .flex()
            .flex_col()
            .child(self.render_title_bar(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(self.render_sidebar(cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .pt_5()
                            .pb_4()
                            .pl_5()
                            .pr_4()
                            .child(div().id("tab-content").flex().flex_col().flex_1().map(
                                |this| match self.active_tab {
                                    AppTab::General => this.child(self.render_general_tab(
                                        record_icon,
                                        record_label,
                                        record_tip,
                                        record_is_idle,
                                        record_disabled,
                                        active_session,
                                        pause_icon,
                                        pause_label,
                                        pause_tip,
                                        pause_disabled,
                                        controls_disabled,
                                        muted_foreground,
                                        cx,
                                    )),
                                    AppTab::Settings => this.child(self.render_settings_tab(
                                        controls_disabled,
                                        muted_foreground,
                                        cx,
                                    )),
                                    AppTab::Cli => {
                                        this.child(self.render_cli_tab(muted_foreground, cx))
                                    }
                                    AppTab::Nerd if self.show_nerd_logs => this.child(
                                        self.render_nerds_tab(metrics_label, muted_foreground, cx),
                                    ),
                                    AppTab::Nerd => this.child(self.render_settings_tab(
                                        controls_disabled,
                                        muted_foreground,
                                        cx,
                                    )),
                                    AppTab::About => {
                                        this.child(self.render_about_tab(muted_foreground, cx))
                                    }
                                },
                            )),
                    ),
            )
            .children(notification_layer)
    }
}

fn switch_on_color(cx: &Context<WrecApp>) -> Hsla {
    if cx.theme().mode.is_dark() {
        Hsla::from(rgb(0x06b6d4))
    } else {
        cx.theme().primary
    }
}

fn permission_state_button(
    status: ScreenRecordingPermissionStatus,
    busy: bool,
    cx: &mut Context<WrecApp>,
) -> UiButton {
    let label = if busy {
        "Checking"
    } else if status.is_granted() {
        "Granted"
    } else {
        "Grant"
    };
    let tooltip = if status.is_granted() {
        "Screen Recording permission granted"
    } else {
        "Grant Screen Recording permission"
    };
    let button = UiButton::new("settings-screen-recording-state")
        .compact()
        .secondary()
        .h(px(CONTROL_HEIGHT))
        .font_weight(FontWeight::SEMIBOLD)
        .label(label)
        .tooltip(tooltip)
        .disabled(busy || status.is_granted())
        .on_click(cx.listener(|this, _, _, cx| {
            this.request_screen_recording_permission(cx);
        }));

    if !busy && !status.is_granted() {
        button.primary()
    } else {
        button
    }
}

fn record_button(
    icon: PhosphorIcon,
    label: &'static str,
    tooltip: &'static str,
    is_idle: bool,
    disabled: bool,
    cx: &mut Context<WrecApp>,
) -> UiButton {
    let theme = cx.theme();
    let button = UiButton::new("record-button")
        .h(px(RECORD_BUTTON_HEIGHT))
        .font_weight(FontWeight::SEMIBOLD)
        .text_base()
        .icon(UiIcon::new(icon).size(px(18.0)).text_color(if is_idle {
            theme.button_primary_foreground
        } else {
            theme.danger_foreground
        }))
        .label(label)
        .tooltip(tooltip)
        .disabled(disabled)
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

fn pause_button(
    icon: PhosphorIcon,
    label: &'static str,
    tooltip: &'static str,
    disabled: bool,
    cx: &mut Context<WrecApp>,
) -> UiButton {
    UiButton::new("pause-button")
        .secondary()
        .h(px(RECORD_BUTTON_HEIGHT))
        .font_weight(FontWeight::SEMIBOLD)
        .text_base()
        .icon(UiIcon::new(icon).size(px(18.0)).text_color(cx.theme().secondary_foreground))
        .label(label)
        .tooltip(tooltip)
        .disabled(disabled)
        .on_click(cx.listener(|this, _, window, cx| {
            this.toggle_pause(window, cx);
            cx.notify();
        }))
}

fn row_label(label: &'static str) -> Div {
    div()
        .text_base()
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}

fn theme_toggle(is_dark: bool, cx: &mut Context<WrecApp>) -> impl IntoElement {
    UiButton::new("theme-mode")
        .ghost()
        .compact()
        .size(px(30.))
        .icon(UiIcon::new(if is_dark {
            PhosphorIcon::Moon
        } else {
            PhosphorIcon::Sun
        }))
        .tooltip(if is_dark {
            "Switch to light mode"
        } else {
            "Switch to dark mode"
        })
        .on_click(cx.listener(move |_, _, window, cx| {
            let mode = if is_dark {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            };
            change_theme(mode, Some(window), cx);
            cx.notify();
        }))
}

fn switch_row(label: &'static str, value: &'static str, value_color: Hsla, switch: Switch) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .h(px(CONTROL_HEIGHT))
        .gap_3()
        .child(
            div()
                .flex()
                .items_baseline()
                .gap_2()
                .min_w(px(0.))
                .child(row_label(label))
                .child(div().text_sm().text_color(value_color).child(value)),
        )
        .child(switch)
}

fn card_section(children: Vec<Div>, cx: &Context<WrecApp>) -> Div {
    let t = cx.theme();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_2p5()
        .rounded_lg()
        .bg(t.group_box)
        .border_1()
        .border_color(t.border)
        .children(children)
}

fn section_label(label: &'static str, muted_foreground: Hsla) -> Div {
    div()
        .text_xs()
        .text_color(muted_foreground)
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}

fn plain_info_row(label: &'static str, value: impl Into<SharedString>, value_color: Hsla) -> Div {
    let value = value.into();

    div()
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .min_h(px(CONTROL_HEIGHT))
        .gap_3()
        .child(row_label(label))
        .child(
            div()
                .min_w(px(0.))
                .text_sm()
                .text_color(value_color)
                .truncate()
                .child(value),
        )
}

pub(crate) fn fps_label(fps: FrameRate) -> &'static str {
    match fps {
        FrameRate::Fps60 => "60 FPS",
        FrameRate::Fps30 => "30 FPS",
    }
}

pub(crate) fn push_app_notification(window: &mut Window, notification: Notification, cx: &mut App) {
    window.push_notification(
        notification
            .w(px(NOTIFICATION_WIDTH))
            .min_h(px(44.))
            .py_2p5()
            .pl_4()
            .pr(px(34.))
            .gap_2(),
        cx,
    );
}

pub(crate) fn configure_notifications(cx: &mut App) {
    let notification = &mut Theme::global_mut(cx).notification;
    notification.placement = Anchor::BottomRight;
    notification.margins.top = px(8.);
    notification.margins.right = px(8.);
    notification.margins.bottom = px(8.);
    notification.margins.left = px(8.);
}

pub(crate) fn change_theme(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    match window {
        Some(window) => {
            Theme::change(mode, Some(&mut *window), cx);
            apply_wrec_theme(cx);
            window.refresh();
        }
        None => {
            Theme::change(mode, None, cx);
            apply_wrec_theme(cx);
        }
    }
}

fn apply_wrec_theme(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    let palette = if theme.mode.is_dark() {
        DARK_PALETTE
    } else {
        LIGHT_PALETTE
    };
    let color = |hex| Hsla::from(rgb(hex));

    theme.font_family = GEIST_FONT_FAMILY.into();
    theme.mono_font_family = GEIST_MONO_FONT_FAMILY.into();
    theme.font_size = px(14.);
    // Flat, clean look: rounded-rectangle corners and no shadows anywhere.
    theme.radius = px(8.);
    theme.radius_lg = px(12.);
    theme.shadow = false;

    theme.background = color(palette.background);
    theme.foreground = color(palette.foreground);
    theme.group_box = color(palette.card);
    theme.group_box_foreground = color(palette.card_foreground);
    theme.popover = color(palette.popover);
    theme.popover_foreground = color(palette.popover_foreground);
    theme.primary = color(palette.primary);
    // Explicit hover/active ramp, following gpui-component's own convention
    // (filled buttons shift their own lightness on interaction) instead of a
    // derived mix — `primary` equals `foreground` here so a mix would be a no-op.
    theme.primary_hover = color(palette.primary_hover);
    theme.primary_active = color(palette.primary_active);
    theme.primary_foreground = color(palette.primary_foreground);
    theme.secondary = color(palette.secondary);
    theme.secondary_hover = color(palette.secondary_hover);
    theme.secondary_active = color(palette.secondary_active);
    theme.secondary_foreground = color(palette.secondary_foreground);
    theme.muted = color(palette.muted);
    theme.muted_foreground = color(palette.muted_foreground);
    theme.accent = color(palette.accent);
    theme.accent_foreground = color(palette.accent_foreground);
    theme.danger = color(palette.destructive);
    theme.danger_hover = theme.danger.mix(theme.background, 0.14);
    theme.danger_active = theme.danger.mix(theme.background, 0.24);
    theme.danger_foreground = color(palette.destructive_foreground);
    theme.border = color(palette.border);
    theme.input = color(palette.input);
    // Neutral focus ring / caret — no colored accent on focused controls.
    theme.ring = theme.border;
    theme.caret = theme.foreground;
    theme.chart_1 = color(palette.chart_1);
    theme.chart_2 = color(palette.chart_2);
    theme.chart_3 = color(palette.chart_3);
    theme.chart_4 = color(palette.chart_4);
    theme.chart_5 = color(palette.chart_5);
    theme.sidebar = color(palette.sidebar);
    theme.sidebar_foreground = color(palette.sidebar_foreground);
    theme.sidebar_primary = color(palette.sidebar_primary);
    theme.sidebar_primary_foreground = color(palette.sidebar_primary_foreground);
    theme.sidebar_accent = color(palette.sidebar_accent);
    theme.sidebar_accent_foreground = color(palette.sidebar_accent_foreground);
    theme.sidebar_border = color(palette.sidebar_border);
    theme.button_primary = theme.primary;
    theme.button_primary_hover = theme.primary_hover;
    theme.button_primary_active = theme.primary_active;
    theme.button_primary_foreground = theme.primary_foreground;
    theme.colors.list = theme.popover;
    theme.list_hover = theme.accent;
    theme.list_active = theme.accent;
    theme.list_active_border = theme.border;
    theme.list_even = theme.popover;
    theme.list_head = theme.muted;
    theme.table = color(palette.card);
    theme.table_head = theme.muted;
    theme.table_head_foreground = theme.muted_foreground;
    theme.table_hover = theme.accent;
    theme.table_active = theme.accent;
    theme.table_active_border = theme.border;
    theme.table_even = color(palette.card);
    theme.table_row_border = theme.border;
    theme.tiles = color(palette.card);
    theme.title_bar = theme.background;
    theme.title_bar_border = theme.border;
    theme.tab_bar = theme.background;
    theme.tab_bar_segmented = theme.muted;
    theme.tab = Hsla::transparent_black();
    theme.tab_active = theme.accent;
    theme.tab_active_foreground = theme.accent_foreground;
    theme.tab_foreground = theme.muted_foreground;
    // Off-state track: `muted` is nearly invisible against the pitch-black
    // background, so use a clearly visible neutral gray in each mode.
    theme.switch = if theme.mode.is_dark() {
        color(0x3f3f46)
    } else {
        color(0xd4d4d8)
    };
    theme.switch_thumb = theme.popover;
    theme.skeleton = theme.muted;
    theme.slider_bar = theme.primary;
    theme.slider_thumb = theme.primary_foreground;
    theme.progress_bar = theme.primary;
    theme.selection = theme.foreground.opacity(0.14);
    theme.link = theme.primary;
    theme.link_hover = theme.primary_hover;
    theme.link_active = theme.primary_active;
}

pub(crate) fn target_key(target: &CaptureTarget) -> String {
    let kind = match target.kind {
        CaptureSourceKind::Display => "display",
        CaptureSourceKind::Window => "window",
    };
    format!("{kind}:{}", target.id)
}

pub(crate) fn metrics_label(metrics: &RecorderMetrics) -> String {
    format!(
        "{}s  {:.1} MB  {:.1} Mbps",
        metrics.elapsed_secs,
        metrics.output_bytes as f32 / 1_000_000.,
        metrics.estimated_bitrate_mbps
    )
}

pub(crate) fn zero_metrics_label() -> String {
    "0s  0.0 MB  0.0 Mbps".to_string()
}

pub(crate) fn resolution_label(resolution: Resolution) -> &'static str {
    match resolution {
        Resolution::Native => "Original",
        Resolution::R720p => "720p",
        Resolution::R1080p => "1080p",
        Resolution::R2k => "2K",
        Resolution::R4k => "4K",
    }
}
