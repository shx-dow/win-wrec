use crate::assets::PhosphorIcon;
use control::DaemonClient;
use gpui::*;
use gpui_component::{
    button::{Button as UiButton, ButtonVariants as _},
    Disableable as _, Icon as UiIcon,
};
use std::sync::{Arc, Mutex};

pub struct ControlWindowState {
    pub paused: bool,
    pub elapsed_secs: u64,
    pub job_id: Option<u64>,
}

pub struct ControlWindow {
    pub state: Arc<Mutex<ControlWindowState>>,
    daemon: DaemonClient,
}

impl ControlWindow {
    pub fn new(state: Arc<Mutex<ControlWindowState>>, daemon: DaemonClient) -> Self {
        Self { state, daemon }
    }
}

impl Render for ControlWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.lock().unwrap();
        let mins = s.elapsed_secs / 60;
        let secs = s.elapsed_secs % 60;
        let timer = format!("{:02}:{:02}", mins, secs);
        let paused = s.paused;
        let job_id = s.job_id;
        drop(s);

        let bg = rgb(0x18181a);
        let fg = rgb(0xf1f1f3);
        let border = rgb(0x2e2e32);
        let red = rgb(0xef4444);

        div()
            .id("wrec-control")
            .size_full()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .gap_2()
            .bg(bg)
            .border_1()
            .border_color(border)
            .rounded_md()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(8.))
                            .h(px(8.))
                            .rounded_full()
                            .bg(red),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child(timer),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(
                        UiButton::new("ctrl-pause")
                            .ghost()
                            .compact()
                            .size(px(30.))
                            .icon(UiIcon::new(if paused {
                                PhosphorIcon::Play
                            } else {
                                PhosphorIcon::Pause
                            }))
                            .disabled(job_id.is_none())
                            .on_click(cx.listener(move |this, _, _, _cx| {
                                let s = this.state.lock().unwrap();
                                let id = s.job_id;
                                let was_paused = s.paused;
                                drop(s);
                                if let Some(job_id) = id {
                                    let daemon = this.daemon.clone();
                                    std::thread::spawn(move || {
                                        if was_paused {
                                            let _ = daemon.resume_job(job_id);
                                        } else {
                                            let _ = daemon.pause_job(job_id);
                                        }
                                    });
                                }
                            })),
                    )
                    .child(
                        UiButton::new("ctrl-stop")
                            .ghost()
                            .compact()
                            .size(px(30.))
                            .icon(UiIcon::new(PhosphorIcon::Stop).text_color(red))
                            .disabled(job_id.is_none())
                            .on_click(cx.listener(move |this, _, _, _cx| {
                                let job_id = this.state.lock().unwrap().job_id;
                                if let Some(job_id) = job_id {
                                    let daemon = this.daemon.clone();
                                    std::thread::spawn(move || {
                                        let _ = daemon.stop_job(job_id);
                                    });
                                }
                            })),
                    ),
            )
    }
}
