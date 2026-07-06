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

#[derive(Clone)]
pub struct AudioSamples {
    pub data: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
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
        let client: IAudioClient =
            wr(device.Activate::<IAudioClient>(CLSCTX_INPROC_SERVER, None))?;
        let format_ptr = wr(client.GetMixFormat())?;
        let fmt = &*format_ptr;

        let is_float = fmt.wFormatTag == 3
            || (fmt.wFormatTag == 65534 && fmt.wBitsPerSample == 32);

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
        let client: IAudioClient =
            wr(device.Activate::<IAudioClient>(CLSCTX_INPROC_SERVER, None))?;

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

        let is_float =
            fmt.wFormatTag == 3 || (fmt.wFormatTag == 65534 && fmt.wBitsPerSample == 32);
        let frame_size = fmt.nBlockAlign as usize;
        let sample_rate = fmt.nSamplesPerSec;
        let channels = fmt.nChannels;
        let bits_per_sample = fmt.wBitsPerSample;

        CoTaskMemFree(Some(format_ptr as *mut _));

        eprintln!(
            "capture-engine: system audio started sample_rate={} channels={}",
            sample_rate, channels
        );

        start_barrier.wait();

        let mut audio_time: i64 = 0;
        let mut first_cycle = true;

        while running.load(Ordering::Relaxed) {
            let mut packet_size = match capture_client.GetNextPacketSize() {
                Ok(s) => s,
                Err(_) => {
                    thread::sleep(Duration::from_millis(10));
                    0
                }
            };

            while packet_size > 0 {
                let mut buffer: *mut u8 = std::ptr::null_mut();
                let mut frames_available: u32 = 0;
                let mut flags: u32 = 0;

                if capture_client
                    .GetBuffer(&mut buffer, &mut frames_available, &mut flags, None, None)
                    .is_err()
                {
                    break;
                }

                let total_bytes = frames_available as usize * frame_size;
                let raw_data = std::slice::from_raw_parts(buffer, total_bytes);

                let (pcm_data, bps) = if is_float {
                    let src = std::slice::from_raw_parts(raw_data.as_ptr() as *const f32, raw_data.len() / 4);
                    let mut out = Vec::with_capacity(src.len() * 2);
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
                    (out, 16)
                } else {
                    (raw_data.to_vec(), bits_per_sample)
                };

                let _ = capture_client.ReleaseBuffer(frames_available);

                let duration = if frame_size > 0 && sample_rate > 0 {
                    (total_bytes as i64 * 10_000_000) / (frame_size as i64 * sample_rate as i64)
                } else {
                    0i64
                };

                if !first_cycle && !paused.load(Ordering::Relaxed) {
                    if let Ok(mut enc) = encoder.lock() {
                        let samples = AudioSamples {
                            data: pcm_data,
                            sample_rate,
                            channels,
                            bits_per_sample: bps,
                        };
                        if let Err(e) = enc.write_audio(&samples, audio_time) {
                            eprintln!("capture-engine: audio write failed: {e}");
                        }
                    }
                }

                audio_time += duration;

                packet_size = match capture_client.GetNextPacketSize() {
                    Ok(s) => s,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(10));
                        break;
                    }
                };
            }

            if first_cycle {
                audio_time = 0;
            }
            first_cycle = false;
            thread::sleep(Duration::from_millis(10));
        }

        let _ = client.Stop();
        eprintln!("capture-engine: system audio stopped");
        CoUninitialize();
    }
    Ok(())
}
