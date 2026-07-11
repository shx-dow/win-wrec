use anyhow::{anyhow, Context as AnyhowContext, Result};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Barrier, Mutex, OnceLock,
};
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::*;

use super::encoder::MfEncoder;

fn wr<T>(r: windows::core::Result<T>) -> Result<T> {
    r.map_err(|e| anyhow!("{e}"))
}

const REFTIMES_PER_SEC: i64 = 10_000_000;
/// Fill silence in ~20ms chunks so the audio timeline stays continuous during idle/pause.
const SILENCE_CHUNK_MS: u64 = 20;

#[derive(Clone, Copy)]
struct MixFormat {
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
    block_align: u16,
    is_float: bool,
}

pub fn query_default_format() -> Result<super::encoder::AudioMediaType> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let enumerator: IMMDeviceEnumerator = wr(CoCreateInstance(
            &MMDeviceEnumerator,
            None,
            CLSCTX_INPROC_SERVER,
        ))?;
        let device = wr(enumerator.GetDefaultAudioEndpoint(eRender, eConsole))?;
        let client: IAudioClient = wr(device.Activate::<IAudioClient>(CLSCTX_INPROC_SERVER, None))?;
        let format_ptr = wr(client.GetMixFormat())?;
        let mix = parse_mix_format(format_ptr)?;

        // Encoder always receives 16-bit PCM.
        let mt = super::encoder::AudioMediaType {
            sample_rate: mix.sample_rate,
            channels: mix.channels,
            bits_per_sample: 16,
            block_align: 2 * mix.channels,
            avg_bytes_per_sec: mix.sample_rate * mix.channels as u32 * 2,
            is_float: false,
        };

        CoTaskMemFree(Some(format_ptr as *mut _));
        CoUninitialize();
        Ok(mt)
    }
}

pub fn spawn_capture_thread(
    encoder: Arc<Mutex<MfEncoder>>,
    paused: Arc<AtomicBool>,
    start_barrier: Arc<Barrier>,
    timeline_start: Arc<OnceLock<Instant>>,
) -> Result<(thread::JoinHandle<()>, Arc<AtomicBool>)> {
    let running = Arc::new(AtomicBool::new(true));
    let thread_running = running.clone();

    let handle = thread::Builder::new()
        .name("wrec-audio".into())
        .spawn(move || {
            if let Err(e) = run_loopback(
                encoder,
                paused,
                thread_running,
                start_barrier,
                timeline_start,
            ) {
                eprintln!("capture-engine: audio capture error: {e}");
            }
        })
        .with_context(|| "failed to spawn audio thread")?;

    Ok((handle, running))
}

unsafe fn parse_mix_format(format: *const WAVEFORMATEX) -> Result<MixFormat> {
    let wave = unsafe { std::ptr::read_unaligned(format) };
    let format_tag = wave.wFormatTag as u32;
    let channels = wave.nChannels;
    let sample_rate = wave.nSamplesPerSec;
    let bits_per_sample = wave.wBitsPerSample;
    let block_align = wave.nBlockAlign;

    let is_float = match format_tag {
        WAVE_FORMAT_IEEE_FLOAT => true,
        WAVE_FORMAT_PCM => false,
        WAVE_FORMAT_EXTENSIBLE => {
            let extensible =
                unsafe { std::ptr::read_unaligned(format.cast::<WAVEFORMATEXTENSIBLE>()) };
            let sub_format = unsafe { std::ptr::addr_of!(extensible.SubFormat).read_unaligned() };
            if sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
                true
            } else if sub_format == KSDATAFORMAT_SUBTYPE_PCM {
                false
            } else {
                // Fallback: 32-bit EXTENSIBLE is almost always float on Windows mix formats.
                bits_per_sample == 32
            }
        }
        _ => bits_per_sample == 32,
    };

    Ok(MixFormat {
        sample_rate,
        channels,
        bits_per_sample,
        block_align,
        is_float,
    })
}

fn convert_to_16bit_pcm(
    raw_data: &[u8],
    is_float: bool,
    bits_per_sample: u16,
    frames: u32,
    channels: u16,
    silent: bool,
) -> Vec<u8> {
    let samples = frames as usize * channels as usize;
    if silent || raw_data.is_empty() {
        return vec![0u8; samples * 2];
    }

    if is_float {
        let src = unsafe { std::slice::from_raw_parts(raw_data.as_ptr() as *const f32, samples) };
        let mut out = Vec::with_capacity(samples * 2);
        for &f in src {
            let s = if f >= 1.0 {
                32767i16
            } else if f <= -1.0 {
                -32768i16
            } else {
                (f * 32767.0) as i16
            };
            out.extend_from_slice(&s.to_le_bytes());
        }
        return out;
    }

    match bits_per_sample {
        16 => {
            let need = samples * 2;
            if raw_data.len() >= need {
                raw_data[..need].to_vec()
            } else {
                raw_data.to_vec()
            }
        }
        24 => {
            let mut out = Vec::with_capacity(samples * 2);
            for chunk in raw_data.chunks_exact(3).take(samples) {
                let value = i32::from_le_bytes([
                    chunk[0],
                    chunk[1],
                    chunk[2],
                    if chunk[2] & 0x80 == 0 { 0 } else { 0xff },
                ]);
                out.extend_from_slice(&((value >> 8) as i16).to_le_bytes());
            }
            out
        }
        32 => {
            let src =
                unsafe { std::slice::from_raw_parts(raw_data.as_ptr() as *const i32, samples) };
            let mut out = Vec::with_capacity(samples * 2);
            for &sample in src {
                out.extend_from_slice(&((sample >> 16) as i16).to_le_bytes());
            }
            out
        }
        _ => vec![0u8; samples * 2],
    }
}

fn hns_from_frames(frames: u32, sample_rate: u32) -> i64 {
    if sample_rate == 0 {
        return 0;
    }
    (frames as i64 * REFTIMES_PER_SEC) / sample_rate as i64
}

fn frames_from_hns(hns: i64, sample_rate: u32) -> u32 {
    if sample_rate == 0 || hns <= 0 {
        return 0;
    }
    ((hns * sample_rate as i64) / REFTIMES_PER_SEC) as u32
}

fn wall_hns(start: Instant) -> i64 {
    let nanos = start.elapsed().as_nanos();
    (nanos / 100) as i64
}

fn write_pcm(encoder: &Arc<Mutex<MfEncoder>>, pcm: &[u8], timestamp: i64, duration: i64) -> bool {
    if pcm.is_empty() || duration <= 0 {
        return true;
    }
    match encoder.lock() {
        Ok(mut enc) => {
            if let Err(e) = enc.write_audio(pcm, timestamp, duration) {
                eprintln!("capture-engine: audio write failed: {e}");
                return false;
            }
            true
        }
        Err(_) => false,
    }
}

fn fill_silence_to(
    encoder: &Arc<Mutex<MfEncoder>>,
    next_hns: &mut i64,
    target_hns: i64,
    sample_rate: u32,
    channels: u16,
) -> bool {
    if target_hns <= *next_hns {
        return true;
    }

    let chunk_frames = ((sample_rate as u64 * SILENCE_CHUNK_MS) / 1000).max(1) as u32;
    let chunk_hns = hns_from_frames(chunk_frames, sample_rate);
    let silence = vec![0u8; chunk_frames as usize * channels as usize * 2];

    while *next_hns < target_hns {
        let remaining = target_hns - *next_hns;
        let (frames, duration) = if remaining >= chunk_hns {
            (chunk_frames, chunk_hns)
        } else {
            let frames = frames_from_hns(remaining, sample_rate).max(1);
            (frames, hns_from_frames(frames, sample_rate))
        };

        let bytes = frames as usize * channels as usize * 2;
        if bytes == silence.len() {
            if !write_pcm(encoder, &silence, *next_hns, duration) {
                return false;
            }
        } else {
            let partial = vec![0u8; bytes];
            if !write_pcm(encoder, &partial, *next_hns, duration) {
                return false;
            }
        }
        *next_hns += duration;
    }
    true
}

fn run_loopback(
    encoder: Arc<Mutex<MfEncoder>>,
    paused: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    start_barrier: Arc<Barrier>,
    timeline_start: Arc<OnceLock<Instant>>,
) -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let enumerator: IMMDeviceEnumerator = wr(CoCreateInstance(
            &MMDeviceEnumerator,
            None,
            CLSCTX_INPROC_SERVER,
        ))?;
        let device = wr(enumerator.GetDefaultAudioEndpoint(eRender, eConsole))?;
        let client: IAudioClient = wr(device.Activate::<IAudioClient>(CLSCTX_INPROC_SERVER, None))?;

        let format_ptr = wr(client.GetMixFormat())?;
        let mix = parse_mix_format(format_ptr)?;

        let buffer_duration = REFTIMES_PER_SEC / 2;
        wr(client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            buffer_duration,
            0,
            format_ptr,
            None,
        ))?;

        let capture_client: IAudioCaptureClient = wr(client.GetService())?;
        wr(client.Start())?;

        let sample_rate = mix.sample_rate;
        let channels = mix.channels;
        let bits_per_sample = mix.bits_per_sample;
        let wasapi_frame_size = mix.block_align as usize;
        let is_float = mix.is_float;

        CoTaskMemFree(Some(format_ptr as *mut _));

        eprintln!(
            "capture-engine: system audio started sample_rate={} channels={} float={}",
            sample_rate, channels, is_float
        );

        // Wait until video side is ready, then share a single wall-clock origin.
        start_barrier.wait();
        let start = loop {
            if let Some(start) = timeline_start.get() {
                break *start;
            }
            thread::sleep(Duration::from_millis(1));
        };

        let mut next_hns: i64 = 0;

        while running.load(Ordering::Relaxed) {
            let mut got_packet = false;

            loop {
                let packet_size = match capture_client.GetNextPacketSize() {
                    Ok(s) => s,
                    Err(_) => break,
                };

                if packet_size == 0 {
                    break;
                }
                got_packet = true;

                let mut buffer: *mut u8 = std::ptr::null_mut();
                let mut frames_available: u32 = 0;
                let mut flags: u32 = 0;

                if capture_client
                    .GetBuffer(&mut buffer, &mut frames_available, &mut flags, None, None)
                    .is_err()
                {
                    break;
                }

                let silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0
                    || buffer.is_null()
                    || paused.load(Ordering::Relaxed);

                let pcm_chunk = if silent {
                    convert_to_16bit_pcm(
                        &[],
                        is_float,
                        bits_per_sample,
                        frames_available,
                        channels,
                        true,
                    )
                } else {
                    let total_bytes = frames_available as usize * wasapi_frame_size;
                    let raw_data = std::slice::from_raw_parts(buffer, total_bytes);
                    convert_to_16bit_pcm(
                        raw_data,
                        is_float,
                        bits_per_sample,
                        frames_available,
                        channels,
                        false,
                    )
                };
                let _ = capture_client.ReleaseBuffer(frames_available);

                if frames_available == 0 || pcm_chunk.is_empty() {
                    continue;
                }

                // Keep the audio timeline continuous: if wall clock advanced past
                // next_hns (e.g. long gap between packets), pad silence first.
                let wall = wall_hns(start);
                if !fill_silence_to(&encoder, &mut next_hns, wall, sample_rate, channels) {
                    running.store(false, Ordering::Relaxed);
                    break;
                }

                let duration = hns_from_frames(frames_available, sample_rate);
                if !write_pcm(&encoder, &pcm_chunk, next_hns, duration) {
                    running.store(false, Ordering::Relaxed);
                    break;
                }
                next_hns += duration;
            }

            // Idle or pause with no WASAPI packets: still advance the audio track
            // with silence so leading quiet periods and pauses land on the timeline.
            if !got_packet || paused.load(Ordering::Relaxed) {
                let wall = wall_hns(start);
                if !fill_silence_to(&encoder, &mut next_hns, wall, sample_rate, channels) {
                    running.store(false, Ordering::Relaxed);
                    break;
                }
            }

            thread::sleep(Duration::from_millis(SILENCE_CHUNK_MS));
        }

        // Final pad so audio duration matches wall-clock stop time.
        let wall = wall_hns(start);
        let _ = fill_silence_to(&encoder, &mut next_hns, wall, sample_rate, channels);

        let _ = client.Stop();
        eprintln!("capture-engine: system audio stopped");
        CoUninitialize();
    }
    Ok(())
}
