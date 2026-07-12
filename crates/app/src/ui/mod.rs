use crate::app::{RecorderState, WrecApp};
use domain::{CaptureSourceKind, Codec, FrameRate, Quality, Resolution};

pub(crate) const WINDOW_SIZE: egui::Vec2 = egui::vec2(520.0, 480.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppTab {
    General,
    Settings,
    Cli,
    About,
    Nerd,
}



pub(crate) fn resolution_disabled(quality: Quality, resolution: Resolution) -> bool {
    quality
        .max_resolution()
        .is_some_and(|cap| resolution.capped_at(cap) != resolution)
}

pub(crate) fn fps_disabled(quality: Quality, fps: FrameRate) -> bool {
    fps.capped_at(quality.max_fps()) != fps
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

pub(crate) fn target_key(target: &domain::CaptureTarget) -> String {
    let kind = match target.kind {
        domain::CaptureSourceKind::Display => "display",
        domain::CaptureSourceKind::Window => "window",
    };
    format!("{kind}:{}", target.id)
}

// ── Entry point ──

pub(crate) fn render(app: &mut WrecApp, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    // Title bar
    egui::TopBottomPanel::top("title_bar")
        .min_height(36.0)
        .show(ctx, |ui| {
            render_title_bar(app, ui);
        });

    // Body
    egui::CentralPanel::default().show(ctx, |ui| {
        match app.active_tab {
            AppTab::General => render_home(app, ui),
            AppTab::Settings => render_settings(app, ui),
            AppTab::Cli => render_cli(app, ui),
            AppTab::Nerd if app.show_nerd_logs => render_nerd(app, ui),
            AppTab::Nerd => render_settings(app, ui),
            AppTab::About => render_about(app, ui),
        }
    });

    // Error dialog
    if app.show_error_dialog {
        egui::Window::new("Error")
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .auto_sized()
            .show(ctx, |ui| {
                ui.label(&app.error_dialog_message);
                ui.horizontal(|ui| {
                    ui.add_space(ui.available_width() - 50.0);
                    if ui.button("OK").clicked() {
                        app.show_error_dialog = false;
                    }
                });
            });
    }

    // Quit dialog
    if app.show_quit_dialog {
        egui::Window::new("Recording in progress")
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .auto_sized()
            .show(ctx, |ui| {
                ui.label("Stop the current recording, save it, and quit Wrec?");
                ui.horizontal(|ui| {
                    if ui.button("Stop recording & quit").clicked() {
                        app.confirm_quit();
                    }
                    if ui.button("Cancel").clicked() {
                        app.show_quit_dialog = false;
                    }
                });
            });
    }

    // Notification toast
    if let Some((ref message, _)) = app.notification_message {
        egui::Area::new(egui::Id::new("notification"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        ui.label(message);
                    });
            });
    }
}

// ── Title bar ──

fn render_title_bar(app: &mut WrecApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.add_space(72.0);

        if app.active_tab != AppTab::General {
            if ui.button("\u{2190} Back").clicked() {
                app.active_tab = AppTab::General;
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Theme toggle
            let theme_label = if app.is_dark_mode { "\u{2600}" } else { "\u{1F319}" };
            if ui.button(theme_label).clicked() {
                app.is_dark_mode = !app.is_dark_mode;
                ui.ctx().set_visuals(if app.is_dark_mode {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
            }

            // Gear menu
            egui::menu::menu_button(ui, "\u{2699}", |ui| {
                if ui.button("Settings").clicked() {
                    app.active_tab = AppTab::Settings;
                    ui.close_menu();
                }
                if ui.button("CLI").clicked() {
                    app.active_tab = AppTab::Cli;
                    ui.close_menu();
                }
                if app.show_nerd_logs {
                    if ui.button("Nerd").clicked() {
                        app.active_tab = AppTab::Nerd;
                        ui.close_menu();
                    }
                }
                if ui.button("About").clicked() {
                    app.active_tab = AppTab::About;
                    ui.close_menu();
                }
            });
        });
    });
}

// ── Home ──

fn render_home(app: &mut WrecApp, ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        // Source & Target row
        ui.horizontal(|ui| {
            ui.scope(|ui| {
                ui.vertical(|ui| {
                    ui.label("Source");
                    let mut source = app.settings.source;
                    egui::ComboBox::from_id_salt("source")
                        .selected_text(app.source_label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut source,
                                CaptureSourceKind::Display,
                                "Display",
                            );
                            ui.selectable_value(
                                &mut source,
                                CaptureSourceKind::Window,
                                "Window",
                            );
                        });
                    if source != app.settings.source {
                        app.set_source(source);
                    }
                });
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label("Target");
                    let targets = app.targets_for_source();
                    let selected_text = app
                        .selected_target_name()
                        .unwrap_or_else(|| "Select...".to_string());
                    egui::ComboBox::from_id_salt("target")
                        .selected_text(selected_text)
                        .show_ui(ui, |ui| {
                            for target in &targets {
                                let key = target_key(target);
                                let is_selected = app
                                    .selected_target_key
                                    .as_deref()
                                    == Some(&key);
                                if ui.selectable_label(is_selected, &target.name).clicked() {
                                    app.set_target_key(key);
                                }
                            }
                        });
                });
            });
        });

        ui.add_space(12.0);

        // Format & Preset row
        ui.horizontal(|ui| {
            ui.scope(|ui| {
                ui.vertical(|ui| {
                    ui.label("Format");
                    let mut codec = app.settings.codec;
                    egui::ComboBox::from_id_salt("format")
                        .selected_text(match codec {
                            Codec::Hevc => "HEVC",
                            Codec::H264 => "H.264",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut codec, Codec::Hevc, "HEVC");
                            ui.selectable_value(&mut codec, Codec::H264, "H.264");
                        });
                    if codec != app.settings.codec {
                        app.set_codec(codec);
                    }
                });
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label("Preset");
                    let mut quality = app.settings.quality;
                    egui::ComboBox::from_id_salt("quality")
                        .selected_text(match quality {
                            Quality::Balanced => "Balanced",
                            Quality::Efficient => "Efficient",
                            Quality::High => "High",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut quality, Quality::Balanced, "Balanced");
                            ui.selectable_value(&mut quality, Quality::Efficient, "Efficient");
                            ui.selectable_value(&mut quality, Quality::High, "High");
                        });
                    if quality != app.settings.quality {
                        app.set_quality(quality);
                    }
                });
            });
        });

        ui.add_space(8.0);

        // Resolution & FPS row
        ui.horizontal(|ui| {
            ui.scope(|ui| {
                ui.vertical(|ui| {
                    ui.label("Resolution");
                    let mut resolution = app.settings.resolution;
                    let res_disabled =
                        |r: Resolution| resolution_disabled(app.settings.quality, r);
                    egui::ComboBox::from_id_salt("resolution")
                        .selected_text(resolution_label(resolution))
                        .show_ui(ui, |ui| {
                            for (res, label) in [
                                (Resolution::Native, "Original"),
                                (Resolution::R4k, "4K"),
                                (Resolution::R2k, "2K"),
                                (Resolution::R1080p, "1080p"),
                                (Resolution::R720p, "720p"),
                            ] {
                                let disabled = res_disabled(res);
                                let resp = ui.add_enabled(!disabled, egui::SelectableLabel::new(
                                    resolution == res,
                                    label,
                                ));
                                if resp.clicked() {
                                    resolution = res;
                                }
                            }
                        });
                    if resolution != app.settings.resolution {
                        app.set_resolution(resolution);
                    }
                });
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label("Frame Rate");
                    let mut fps = app.settings.fps;
                    let fps_disabled = |f: FrameRate| {
                        f.capped_at(app.settings.quality.max_fps()) != f
                    };
                    egui::ComboBox::from_id_salt("fps")
                        .selected_text(match fps {
                            FrameRate::Fps60 => "60 FPS",
                            FrameRate::Fps30 => "30 FPS",
                        })
                        .show_ui(ui, |ui| {
                            for (fr, label) in [
                                (FrameRate::Fps60, "60 FPS"),
                                (FrameRate::Fps30, "30 FPS"),
                            ] {
                                let disabled = fps_disabled(fr);
                                let resp = ui.add_enabled(!disabled, egui::SelectableLabel::new(
                                    fps == fr,
                                    label,
                                ));
                                if resp.clicked() {
                                    fps = fr;
                                }
                            }
                        });
                    if fps != app.settings.fps {
                        app.set_fps(fps);
                    }
                });
            });
        });

        ui.add_space(8.0);

        // Cursor & Audio toggles
        ui.horizontal(|ui| {
            let mut cursor = app.settings.include_cursor;
            if ui.checkbox(&mut cursor, "Cursor").changed() {
                app.set_include_cursor(cursor);
            }
            ui.add_space(16.0);
            let mut audio = app.settings.include_system_audio;
            if ui.checkbox(&mut audio, "System Audio").changed() {
                app.set_include_system_audio(audio);
            }
        });

        // Push record button to center of remaining space
        ui.add_space(ui.available_height() * 0.25);

        // Record / Stop button
        let is_active = app.recorder_state.is_active_session();
        let show_pause = app.recorder_state.is_recording() || app.recorder_state.is_paused();
        let record_label = if is_active {
            if app.recorder_state.is_paused() {
                "Resume"
            } else {
                "Stop"
            }
        } else {
            "Record"
        };
        let record_disabled = matches!(
            app.recorder_state,
            RecorderState::Starting
                | RecorderState::Pausing
                | RecorderState::Resuming
                | RecorderState::Stopping
        ) || (!is_active
            && (app.permission_busy || !app.permission_status.is_granted()));

        if show_pause {
            ui.horizontal(|ui| {
                ui.add_space(ui.available_width() * 0.25);
                let pause_label = if app.recorder_state.is_paused() {
                    "Resume"
                } else {
                    "Pause"
                };
                let pause_disabled = matches!(
                    app.recorder_state,
                    RecorderState::Pausing
                        | RecorderState::Resuming
                        | RecorderState::Stopping
                );
                if ui
                    .add_enabled(
                        !pause_disabled,
                        egui::Button::new(pause_label).min_size(egui::vec2(110.0, 42.0)),
                    )
                    .clicked()
                {
                    app.toggle_pause();
                }
                if ui
                    .add_enabled(
                        !record_disabled,
                        egui::Button::new(record_label).min_size(egui::vec2(110.0, 42.0)),
                    )
                    .clicked()
                {
                    app.toggle_recording();
                }
            });
        } else {
            if ui
                .add_enabled(
                    !record_disabled,
                    egui::Button::new(record_label).min_size(egui::vec2(240.0, 42.0)),
                )
                .clicked()
            {
                app.toggle_recording();
            }
        }

        ui.add_space(4.0);
        ui.label(&app.status);
    });
}

// ── Settings ──

fn render_settings(app: &mut WrecApp, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.vertical(|ui| {
            // Screen Recording permission
            ui.horizontal(|ui| {
                ui.label("Screen Recording");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if app.permission_busy {
                        "Checking"
                    } else if app.permission_status.is_granted() {
                        "Granted"
                    } else {
                        "Grant"
                    };
                    if ui
                        .button(label)
                        .on_disabled_hover_text("Already granted")
                        .clicked()
                    {
                        app.request_screen_recording_permission();
                    }
                });
            });

            ui.separator();

            // Hide wrec
            ui.horizontal(|ui| {
                let mut hide = app.settings.hide_wrec;
                if ui.checkbox(&mut hide, "Hide wrec from recording").changed() {
                    app.set_hide_wrec(hide);
                }
            });

            // Logs
            ui.horizontal(|ui| {
                let mut logs = app.show_nerd_logs;
                if ui.checkbox(&mut logs, "Show Nerd tab").changed() {
                    app.set_show_nerd_logs(logs);
                }
            });

            ui.separator();

            // Output path
            ui.horizontal(|ui| {
                ui.label("Output folder");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Open").clicked() {
                        app.open_last_recording_dir();
                    }
                    if ui.button("Choose").clicked() {
                        app.choose_output_dir_interactive();
                    }
                });
            });
            let mut output = app.settings.output_dir.display().to_string();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut output)
                        .hint_text("Output folder"),
                )
                .changed()
            {
                let path = std::path::PathBuf::from(output.trim());
                if !output.trim().is_empty() {
                    app.set_output_dir(path);
                }
            }

            // Refresh targets
            ui.separator();
            if ui.button("Refresh capture targets").clicked() {
                app.refresh_targets();
            }
        });
    });
}

// ── CLI ──

fn render_cli(app: &mut WrecApp, ui: &mut egui::Ui) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label("Status");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(app.cli_install_status.label());
                if ui.button("\u{21BB}").clicked() {
                    app.refresh_cli_install_status();
                }
            });
        });

        ui.separator();

        if app.cli_install_command().is_some() {
            ui.horizontal(|ui| {
                ui.label("Install command");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Copy").clicked() {
                        app.copy_cli_install_command();
                        if let Some(cmd) = app.cli_install_command() {
                            ui.ctx().copy_text(cmd);
                        }
                    }
                });
            });
        }
    });
}

// ── Nerd ──

fn render_nerd(app: &mut WrecApp, ui: &mut egui::Ui) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label("Logs");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Open").clicked() {
                    app.open_recordings_data_dir();
                }
            });
        });

        ui.separator();

        // Metrics
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading(&app.metrics_label());
        });

        ui.separator();

        // Log entries
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                for log in app.logs.iter().rev().take(20) {
                    ui.label(log);
                }
            });
    });
}

// ── About ──

fn render_about(app: &mut WrecApp, ui: &mut egui::Ui) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label("Version");
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.label(env!("CARGO_PKG_VERSION"));
                },
            );
        });

        ui.separator();

        if ui.button("GitHub").clicked() {
            app.open_url(crate::app::GITHUB_URL);
        }
    });
}
