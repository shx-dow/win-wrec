use anyhow::{anyhow, Context as AnyhowContext, Result};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Barrier, Mutex,
};
use std::thread;
use std::time::Duration;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

use super::encoder::MfEncoder;

fn wr<T>(r: windows::core::Result<T>) -> Result<T> {
    r.map_err(|e| anyhow!("{e}"))
}

const REFTIMES_PER_SEC: i64 = 10_000_000;

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
        let fmt = &*format_ptr;

        let is_float = fmt.wFormatTag == 3 || (fmt.wFormatTag == 65534 && fmt.wBitsPerSample == 32);

        let mt = if is_float {
            super::encoder::AudioMediaType {
                sample_rate: fmt.nSamplesPerSec,
                channels: fmt.nChannels,
                bits_per_sample: 16,
                block_align: 2 * fmt.nChannels,
                avg_bytes_per_sec: fmt.nSamplesPerSec * fmt.nChannels as u32 * 2,
                is_float: false,
            }
        } else {
            super::encoder::AudioMediaType {
                sample_rate: fmt.nSamplesPerSec,
                channels: fmt.nChannels,
                bits_per_sample: fmt.wBitsPerSample,
                block_align: fmt.nBlockAlign,
                avg_bytes_per_sec: fmt.nAvgBytesPerSec,
                is_float: false,
            }
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
) -> Result<(thread::JoinHandle<()>, Arc<AtomicBool>)> {
    let running = Arc::new(AtomicBool::new(true));
    let thread_running = running.clone();

    let handle = thread::Builder::new()
        .name("wrec-audio".into())
        .spawn(move || {
            if let Err(e) = run_loopback(encoder, paused, thread_running, start_barrier) {
                eprintln!("capture-engine: audio capture error: {e}");
            }
        })
        .with_context(|| "failed to spawn audio thread")?;

    Ok((handle, running))
}

fn convert_to_16bit_pcm(raw_data: &[u8], is_float: bool, bits_per_sample: u16, _channels: u16) -> Vec<u8> {
    if is_float {
        let samples = raw_data.len() / 4;
        let src = unsafe { std::slice::from_raw_parts(raw_data.as_ptr() as *const f32, samples) };
        let mut out = Vec::with_capacity(samples * 2);
        for &f in src {
            let s = if f >= 1.0 {
                32767i16
            } else if f <= -1.0 {
                -32768i16
            } else {
                (f * 32768.0) as i16
            };
            out.extend_from_slice(&s.to_le_bytes());
        }
        return out;
    }

    match bits_per_sample {
        16 => raw_data.to_vec(),
        24 => {
            let samples = raw_data.len() / 3;
            let mut out = Vec::with_capacity(samples * 2);
            for chunk in raw_data.chunks_exact(3) {
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
            let samples = raw_data.len() / 4;
            let src = unsafe { std::slice::from_raw_parts(raw_data.as_ptr() as *const i32, samples) };
            let mut out = Vec::with_capacity(samples * 2);
            for &sample in src {
                out.extend_from_slice(&((sample >> 16) as i16).to_le_bytes());
            }
            out
        }
        _ => raw_data.to_vec(),
    }
}

fn run_loopback(
    encoder: Arc<Mutex<MfEncoder>>,
    paused: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    start_barrier: Arc<Barrier>,
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
        let fmt = &*format_ptr;

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

        let is_float = fmt.wFormatTag == 3 || (fmt.wFormatTag == 65534 && fmt.wBitsPerSample == 32);
        let sample_rate = fmt.nSamplesPerSec;
        let channels = fmt.nChannels;
        let bits_per_sample = fmt.wBitsPerSample;
        let wasapi_frame_size = fmt.nBlockAlign as usize;

        CoTaskMemFree(Some(format_ptr as *mut _));

        eprintln!(
            "capture-engine: system audio started sample_rate={} channels={}",
            sample_rate, channels
        );

        start_barrier.wait();

        let mut base_device_position: Option<u64> = None;

        while running.load(Ordering::Relaxed) {
            loop {
                let packet_size = match capture_client.GetNextPacketSize() {
                    Ok(s) => s,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(10));
                        break;
                    }
                };

                if packet_size == 0 {
                    break;
                }

                let mut buffer: *mut u8 = std::ptr::null_mut();
                let mut frames_available: u32 = 0;
                let mut flags: u32 = 0;
                let mut dev_position: u64 = 0;

                if capture_client
                    .GetBuffer(
                        &mut buffer,
                        &mut frames_available,
                        &mut flags,
                        Some(&mut dev_position),
                        None,
                    )
                    .is_err()
                {
                    break;
                }

                let total_bytes = frames_available as usize * wasapi_frame_size;
                let raw_data = std::slice::from_raw_parts(buffer, total_bytes);

                let pcm_chunk = convert_to_16bit_pcm(raw_data, is_float, bits_per_sample, channels);
                let _ = capture_client.ReleaseBuffer(frames_available);

                if paused.load(Ordering::Relaxed) || pcm_chunk.is_empty() {
                    continue;
                }

                let base = *base_device_position.get_or_insert(dev_position);
                let rel = dev_position.saturating_sub(base);
                let ts = (rel as i64 * REFTIMES_PER_SEC) / sample_rate as i64;
                let duration =
                    (frames_available as i64 * REFTIMES_PER_SEC) / sample_rate as i64;

                if let Ok(mut enc) = encoder.lock() {
                    if let Err(e) = enc.write_audio(&pcm_chunk, ts, duration) {
                        eprintln!("capture-engine: audio write failed: {e}");
                    }
                }
            }
            thread::sleep(Duration::from_millis(10));
        }

        let _ = client.Stop();
        eprintln!("capture-engine: system audio stopped");
        CoUninitialize();
    }
    Ok(())
}
