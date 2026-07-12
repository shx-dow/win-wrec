mod app;
mod platform;
mod ui;

use app::WrecApp;
use ui::WINDOW_SIZE;

fn main() {
    init_tracing();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(WINDOW_SIZE)
            .with_min_inner_size(WINDOW_SIZE)
            .with_resizable(false)
            .with_decorations(true)
            .with_title("wrec"),
        ..Default::default()
    };

    eframe::run_native(
        "wrec",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::light());
            let app = WrecApp::new();
            // On first update, store the native window handle
            Ok(Box::new(app))
        }),
    )
    .expect("failed to run wrec");
}

fn init_tracing() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let path = config::log_path();
    if let Err(err) = std::fs::create_dir_all(path.parent().unwrap_or(&path)) {
        eprintln!("failed to create log directory: {err}");
    }

    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(file) => tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_ansi(false)
            .with_writer(move || file.try_clone().expect("clone log file"))
            .init(),
        Err(err) => {
            eprintln!("failed to open log file {}: {err}", path.display());
            tracing_subscriber::fmt().with_env_filter(env_filter).init();
        }
    }
}
