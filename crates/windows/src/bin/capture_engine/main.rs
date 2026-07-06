#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("capture-engine: unsupported: Windows capture engine only runs on Windows");
    std::process::exit(1);
}

#[cfg(target_os = "windows")]
fn main() {
    if let Err(err) = run() {
        eprintln!("capture-engine: error: {err:#}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "windows")]
mod wgc_backend;

#[cfg(target_os = "windows")]
mod raw;

#[cfg(target_os = "windows")]
pub(crate) enum Backend {
    WindowsCapture,
    Raw,
}

#[cfg(target_os = "windows")]
pub(crate) struct RecordArgs {
    pub output_path: std::path::PathBuf,
    pub fps: u32,
    pub include_cursor: bool,
    pub target_kind: String,
    pub target_id: u64,
    pub codec: String,
    pub quality: String,
    pub resolution: String,
    pub include_system_audio: bool,
    pub hide_wrec: bool,
    pub backend: Backend,
}

#[cfg(target_os = "windows")]
fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if matches!(args.get(1).map(String::as_str), Some("--permission-status")) {
        print_permission_status()?;
        return Ok(());
    }

    if matches!(
        args.get(1).map(String::as_str),
        Some("--request-permission")
    ) {
        print_permission_status()?;
        return Ok(());
    }

    if matches!(args.get(1).map(String::as_str), Some("--list")) {
        ensure_capture_supported()?;
        wgc_backend::list_targets()?;
        return Ok(());
    }

    let record_args = parse_record_args(&args)?;

    match record_args.backend {
        Backend::WindowsCapture => {
            eprintln!(
                "capture-engine: target={} id={} fps={} cursor={} codec={} quality={} resolution={} pipeline=wgc-mediafoundation",
                record_args.target_kind,
                record_args.target_id,
                record_args.fps,
                record_args.include_cursor,
                record_args.codec,
                record_args.quality,
                record_args.resolution,
            );
            ensure_capture_supported()?;

            if record_args.include_system_audio {
                eprintln!("capture-engine: system audio capture requested via WASAPI loopback");
            }
            if record_args.hide_wrec {
                eprintln!("capture-engine: hide_wrec requested; relying on owner-process window affinity for Wrec app windows");
            }

            wgc_backend::start_recording(record_args)
        }
        Backend::Raw => {
            eprintln!(
                "capture-engine: target={} id={} fps={} cursor={} codec={} quality={} resolution={} pipeline=raw-dxgi-mediafoundation",
                record_args.target_kind,
                record_args.target_id,
                record_args.fps,
                record_args.include_cursor,
                record_args.codec,
                record_args.quality,
                record_args.resolution,
            );
            raw::start_recording(record_args)
        }
    }
}

#[cfg(target_os = "windows")]
fn print_permission_status() -> anyhow::Result<()> {
    if wgc_backend::is_capture_supported()? {
        println!("granted");
    } else {
        println!("missing");
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn ensure_capture_supported() -> anyhow::Result<()> {
    if wgc_backend::is_capture_supported()? {
        Ok(())
    } else {
        anyhow::bail!("Windows Graphics Capture is not supported on this Windows build")
    }
}

#[cfg(target_os = "windows")]
fn parse_record_args(args: &[String]) -> anyhow::Result<RecordArgs> {
    use anyhow::{bail, Context as AnyhowContext};

    let usage = "usage: capture-engine <output-path> <fps> <include-cursor> <display|window> <id> <hevc|h264> <efficient|balanced|high> <native|720p|1080p|2k|4k> [include-system-audio] [hide-wrec] [backend]";
    if args.len() < 9 {
        bail!("{usage}");
    }

    let backend = match args.get(11).map(|s| s.as_str()).or_else(|| {
        std::env::var("WREC_CAPTURE_BACKEND")
            .ok()
            .map(|s| Box::leak(s.into_boxed_str()) as &str)
    }) {
        Some("raw") => Backend::Raw,
        _ => Backend::WindowsCapture,
    };

    Ok(RecordArgs {
        output_path: std::path::PathBuf::from(&args[1]),
        fps: args[2].parse::<u32>().unwrap_or(30).clamp(1, 120),
        include_cursor: args[3] == "true",
        target_kind: args[4].to_lowercase(),
        target_id: args[5]
            .parse::<u64>()
            .with_context(|| format!("invalid target id `{}`", args[5]))?,
        codec: args[6].to_lowercase(),
        quality: args[7].to_lowercase(),
        resolution: args[8].to_lowercase(),
        include_system_audio: args.get(9).is_some_and(|arg| arg == "true"),
        hide_wrec: args.get(10).map_or(true, |arg| arg == "true"),
        backend,
    })
}
