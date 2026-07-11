use anyhow::{Context as AnyhowContext, Result};
use std::io::BufRead;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Barrier, Mutex, OnceLock,
};
use std::thread;
use std::time::{Duration, Instant};

use crate::RecordArgs;

mod audio;
mod dxgi;
mod encoder;

use dxgi::DxgiCapture;
use encoder::MfEncoder;

enum CaptureCommand {
    Pause,
    Resume,
    Stop,
}

pub fn start_recording(args: RecordArgs) -> Result<()> {
    let _ = unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        )
    };

    let target = domain::CaptureTarget {
        id: args.target_id,
        name: String::new(),
        kind: match args.target_kind.as_str() {
            "window" => domain::CaptureSourceKind::Window,
            _ => domain::CaptureSourceKind::Display,
        },
    };

    let mut dxgi = DxgiCapture::new(&target).with_context(|| "DXGI init")?;
    let video_media_type = dxgi.media_type();

    let audio_mt = if args.include_system_audio {
        Some(audio::query_default_format().with_context(|| "query audio format")?)
    } else {
        None
    };

    let codec = match args.codec.as_str() {
        "h264" => domain::Codec::H264,
        _ => domain::Codec::Hevc,
    };
    let quality = match args.quality.as_str() {
        "efficient" => domain::Quality::Efficient,
        "high" => domain::Quality::High,
        _ => domain::Quality::Balanced,
    };

    // Placeholder; real t=0 is set after the A/V start barrier so both streams share it.
    let timeline_start = Arc::new(OnceLock::new());
    let encoder = MfEncoder::new(
        &args.output_path,
        &video_media_type,
        audio_mt.as_ref(),
        args.fps,
        quality,
        codec,
        Instant::now(),
    )
    .with_context(|| "encoder init")?;
    let encoder = Arc::new(Mutex::new(encoder));

    let paused = Arc::new(AtomicBool::new(false));

    let start_barrier = args.include_system_audio.then(|| Arc::new(Barrier::new(2)));

    let audio_handle = if let Some(barrier) = &start_barrier {
        let enc = encoder.clone();
        let p = paused.clone();
        let b = barrier.clone();
        let start = timeline_start.clone();
        let (handle, thread_running) =
            audio::spawn_capture_thread(enc, p, b, start).with_context(|| "spawn audio thread")?;
        Some((handle, thread_running))
    } else {
        None
    };

    let target_fps = args.fps;
    let frame_duration = Duration::from_micros(1_000_000 / target_fps as u64);

    eprintln!("capture-engine: recording started");

    let (cmd_tx, cmd_rx) = mpsc::channel();
    let commands = cmd_rx;
    thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines().map_while(std::result::Result::ok) {
            let command = match line.trim().to_lowercase().as_str() {
                "pause" => CaptureCommand::Pause,
                "resume" => CaptureCommand::Resume,
                "stop" => CaptureCommand::Stop,
                _ => continue,
            };
            if cmd_tx.send(command).is_err() {
                return;
            }
        }
        let _ = cmd_tx.send(CaptureCommand::Stop);
    });

    if let Some(barrier) = &start_barrier {
        barrier.wait();
    }

    // Shared A/V origin: first sample on both streams is relative to this instant.
    let recording_start = Instant::now();
    let _ = timeline_start.set(recording_start);
    if let Ok(mut enc) = encoder.lock() {
        enc.set_recording_start(recording_start);
    }

    let mut next_frame_time = recording_start;
    let metric_start = recording_start;
    let mut frame_count: u64 = 0;

    loop {
        match commands.try_recv() {
            Ok(CaptureCommand::Pause) => {
                paused.store(true, Ordering::Relaxed);
                eprintln!("capture-engine: recording paused");
            }
            Ok(CaptureCommand::Resume) => {
                paused.store(false, Ordering::Relaxed);
                eprintln!("capture-engine: recording resumed");
            }
            Ok(CaptureCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => {
                break;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }

        let now = Instant::now();
        if now < next_frame_time {
            thread::sleep(next_frame_time - now);
        }
        next_frame_time += frame_duration;
        // A slow capture/encode must drop schedule slots, not spin at full CPU
        // trying to replay every missed frame deadline.
        if next_frame_time < Instant::now() {
            next_frame_time = Instant::now() + frame_duration;
        }

        if paused.load(Ordering::Relaxed) {
            continue;
        }

        match dxgi.acquire_frame() {
            Ok(mut frame_data) => {
                if !args.include_cursor {
                    frame_data.cursor = None;
                }
                if let Ok(mut enc) = encoder.lock() {
                    if let Err(e) = enc.write_video(&frame_data) {
                        eprintln!("capture-engine: video write failed: {e}");
                    }
                }
                dxgi.release_frame();
                frame_count += 1;
            }
            Err(e) => {
                if dxgi.is_timeout(&e) {
                    continue;
                }
                eprintln!("capture-engine: DXGI frame acquire failed: {e:#}");
                break;
            }
        }

        let elapsed = metric_start.elapsed().as_secs();
        if elapsed > 0 && frame_count % (target_fps as u64 * 5) == 0 {
            let metadata = std::fs::metadata(&args.output_path).ok();
            let output_bytes = metadata.map(|m| m.len()).unwrap_or(0);
            let estimated_bitrate = if elapsed > 0 {
                output_bytes as f32 * 8.0 / elapsed as f32 / 1_000_000.0
            } else {
                0.0
            };
            eprintln!(
                "capture-engine: metrics elapsed={} frames={} size={} bitrate={:.1}mbps",
                elapsed, frame_count, output_bytes, estimated_bitrate
            );
        }
    }

    eprintln!("capture-engine: recording finished frames={frame_count}");

    if let Some((handle, running)) = audio_handle {
        running.store(false, Ordering::Relaxed);
        let _ = handle.join();
    }

    drop(dxgi);
    if let Ok(mut enc) = encoder.lock() {
        enc.finalize().with_context(|| "encoder finalize")?;
    }

    unsafe {
        windows::Win32::System::Com::CoUninitialize();
    }

    Ok(())
}
