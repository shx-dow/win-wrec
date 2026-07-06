use anyhow::{Context as AnyhowContext, Result};
use std::io::BufRead;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use std::thread;

use crate::RecordArgs;

mod dxgi;
mod audio;
mod encoder;

use dxgi::DxgiCapture;
use audio::WasapiCapture;
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

    let mut dxgi = DxgiCapture::new(&target)
        .with_context(|| "DXGI init")?;
    let mut audio = WasapiCapture::new(args.include_system_audio)
        .with_context(|| "WASAPI init")?;
    let video_media_type = dxgi.media_type();
    let audio_media_type = audio.media_type();

    let codec = match args.codec.as_str() {
        "h264" => domain::Codec::H264,
        _ => domain::Codec::Hevc,
    };
    let quality = match args.quality.as_str() {
        "efficient" => domain::Quality::Efficient,
        "high" => domain::Quality::High,
        _ => domain::Quality::Balanced,
    };

    let mut encoder = MfEncoder::new(
        &args.output_path,
        &video_media_type,
        audio_media_type.as_ref(),
        args.fps,
        quality,
        codec,
    ).with_context(|| "encoder init")?;

    let target_fps = args.fps;
    let frame_duration = Duration::from_micros(1_000_000 / target_fps as u64);
    let mut next_frame_time = Instant::now();

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

    let metric_start = Instant::now();
    let mut frame_count: u64 = 0;
    let mut paused = false;

    loop {
        match commands.try_recv() {
            Ok(CaptureCommand::Pause) => {
                paused = true;
                eprintln!("capture-engine: recording paused");
            }
            Ok(CaptureCommand::Resume) => {
                paused = false;
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

        if paused {
            let _ = audio.poll();
            continue;
        }

        if let Some(audio_samples) = audio.poll() {
            if let Err(e) = encoder.write_audio(&audio_samples) {
                eprintln!("capture-engine: audio write failed: {e}");
            }
        } else if args.include_system_audio {
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
                    eprintln!("capture-engine: silence write failed: {e}");
                }
            }
        }

        match dxgi.acquire_frame() {
            Ok(frame_data) => {
                if let Err(e) = encoder.write_video(&frame_data) {
                    eprintln!("capture-engine: video write failed: {e}");
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

    drop(dxgi);
    drop(audio);
    encoder.finalize().with_context(|| "encoder finalize")?;

    unsafe { windows::Win32::System::Com::CoUninitialize(); }

    Ok(())
}
