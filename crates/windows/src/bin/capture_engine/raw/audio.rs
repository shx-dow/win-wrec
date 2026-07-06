use anyhow::{anyhow, Result};
use std::mem;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

fn wr<T>(r: windows::core::Result<T>) -> Result<T> {
    r.map_err(|e| anyhow!("{e}"))
}

const REFTIMES_PER_SEC: i64 = 10_000_000;

pub struct WasapiCapture {
    client: Option<IAudioClient>,
    capture_client: Option<IAudioCaptureClient>,
    wave_format_bytes: Option<Vec<u8>>,
    buffer_frames: u32,
}

#[derive(Clone)]
pub struct AudioSamples {
    pub data: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
}

impl WasapiCapture {
    pub fn new(include_system_audio: bool) -> Result<Self> {
        if !include_system_audio {
            return Ok(Self { client: None, capture_client: None, wave_format_bytes: None, buffer_frames: 0 });
        }

        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

        let enumerator: IMMDeviceEnumerator = wr(unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)
        })?;

        let device = wr(unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) })?;
        let client: IAudioClient = wr(unsafe { device.Activate::<IAudioClient>(CLSCTX_INPROC_SERVER, None) })?;

        let format_ptr = wr(unsafe { client.GetMixFormat() })?;
        let cb_size = unsafe { (*format_ptr).cbSize as usize };
        let total_size = mem::size_of::<WAVEFORMATEX>() + cb_size;
        let wave_format_bytes = unsafe {
            let mut buf = vec![0u8; total_size];
            std::ptr::copy_nonoverlapping(format_ptr as *const u8, buf.as_mut_ptr(), total_size);
            buf
        };
        unsafe { CoTaskMemFree(Some(format_ptr as *mut _)); }

        let fmt = wave_format_bytes.as_ptr() as *const WAVEFORMATEX;

        let buffer_duration = REFTIMES_PER_SEC / 2;
        wr(unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                buffer_duration,
                0,
                fmt,
                None,
            )
        })?;

        let buffer_frames = unsafe { client.GetBufferSize().unwrap_or(0) };
        let capture_client: IAudioCaptureClient = wr(unsafe { client.GetService() })?;
        wr(unsafe { client.Start() })?;

        let fmt_ref = unsafe { &*fmt };
        let sr = fmt_ref.nSamplesPerSec;
        let ch = fmt_ref.nChannels;
        eprintln!("capture-engine: system audio started sample_rate={sr} channels={ch}");

        Ok(Self { client: Some(client), capture_client: Some(capture_client), wave_format_bytes: Some(wave_format_bytes), buffer_frames })
    }

    pub fn media_type(&self) -> Option<super::encoder::AudioMediaType> {
        self.wave_format_bytes.as_ref().map(|bytes| {
            let fmt = unsafe { &*(bytes.as_ptr() as *const WAVEFORMATEX) };
            let is_float = fmt.wFormatTag == 3 || (fmt.wFormatTag == 65534 && fmt.wBitsPerSample == 32);
            if is_float {
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
            }
        })
    }

    pub fn channels(&self) -> u16 {
        self.wave_format_bytes.as_ref().map_or(0, |b| {
            let fmt = unsafe { &*(b.as_ptr() as *const WAVEFORMATEX) };
            fmt.nChannels
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.wave_format_bytes.as_ref().map_or(0, |b| {
            let fmt = unsafe { &*(b.as_ptr() as *const WAVEFORMATEX) };
            fmt.nSamplesPerSec
        })
    }

    pub fn poll(&mut self) -> Option<AudioSamples> {
        let capture = self.capture_client.as_ref()?;
        unsafe {
            if capture.GetNextPacketSize().ok().unwrap_or(0) == 0 { return None; }

            let mut buffer: *mut u8 = std::ptr::null_mut();
            let mut frames_available: u32 = 0;
            let mut flags: u32 = 0;
            let mut _device_position: u64 = 0;
            let mut _qpc_position: u64 = 0;

            if capture
                .GetBuffer(
                    &mut buffer,
                    &mut frames_available,
                    &mut flags,
                    Some(&mut _device_position),
                    Some(&mut _qpc_position),
                )
                .is_err()
            {
                return None;
            }

            let fmt = self.wave_format_bytes.as_ref().map(|b| b.as_ptr() as *const WAVEFORMATEX)?;
            let fmt_ref = &*fmt;
            let frame_size = fmt_ref.nBlockAlign as usize;
            let total_bytes = frames_available as usize * frame_size;
            let raw_data = std::slice::from_raw_parts(buffer, total_bytes);

            let is_float = fmt_ref.wFormatTag == 3 || (fmt_ref.wFormatTag == 65534 && fmt_ref.wBitsPerSample == 32);

            let (data, bits_per_sample) = if is_float {
                let src = std::slice::from_raw_parts(raw_data.as_ptr() as *const f32, raw_data.len() / 4);
                let mut pcm = Vec::with_capacity(src.len() * 2);
                for &f in src {
                    let sample = if f >= 1.0 { 32767i16 } else if f <= -1.0 { -32768i16 } else { (f * 32768.0) as i16 };
                    pcm.extend_from_slice(&sample.to_le_bytes());
                }
                (pcm, 16)
            } else {
                (raw_data.to_vec(), fmt_ref.wBitsPerSample)
            };

            let _ = capture.ReleaseBuffer(frames_available);

            Some(AudioSamples {
                data,
                sample_rate: fmt_ref.nSamplesPerSec,
                channels: fmt_ref.nChannels,
                bits_per_sample,
            })
        }
    }
}

impl Drop for WasapiCapture {
    fn drop(&mut self) {
        if let Some(client) = &self.client {
            unsafe { let _ = client.Stop(); let _ = client.Reset(); }
        }
    }
}
