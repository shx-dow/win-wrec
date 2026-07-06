use anyhow::{anyhow, Result};
use domain::{Codec, Quality};
use std::path::Path;
use std::ptr;
use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::*;

fn wr<T>(r: windows::core::Result<T>) -> Result<T> {
    r.map_err(|e| anyhow!("{e}"))
}

pub struct VideoMediaType {
    pub width: u32,
    pub height: u32,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
}

pub struct AudioMediaType {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub block_align: u16,
    pub avg_bytes_per_sec: u32,
    pub is_float: bool,
}

pub struct MfEncoder {
    sink_writer: Option<IMFSinkWriter>,
    video_stream_index: u32,
    audio_stream_index: Option<u32>,
    width: u32,
    height: u32,
    fps: u32,
    started: bool,
    finalized: bool,
    video_sample_count: u64,
    video_start_time: Option<std::time::Instant>,
    audio_time: i64,
}

impl MfEncoder {
    pub fn new(
        output_path: &Path,
        video_mt: &VideoMediaType,
        audio_mt: Option<&AudioMediaType>,
        fps: u32,
        quality: Quality,
        codec: Codec,
    ) -> Result<Self> {
        if wr(unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }).is_err() {
            return Err(anyhow!("MFStartup failed"));
        }

        let output_path_str = output_path.to_string_lossy();
        let output_wide: Vec<u16> = std::os::windows::ffi::OsStrExt::encode_wide(
            std::ffi::OsStr::new(&output_path_str.as_ref()),
        )
        .chain(std::iter::once(0))
        .collect();

        let attrs = Self::create_sink_writer_attributes()?;

        let sink_writer: IMFSinkWriter = wr(unsafe {
            MFCreateSinkWriterFromURL(
                windows::core::PCWSTR::from_raw(output_wide.as_ptr()),
                None,
                Some(&attrs),
            )
        })?;

        let width = video_mt.width;
        let height = video_mt.height;
        let bitrate = Self::target_bitrate(width, height, fps, quality, codec);

        let video_output_type: IMFMediaType = {
            let mt = wr(unsafe { MFCreateMediaType() })?;
            wr(unsafe { mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) })?;
            let codec_guid = match codec {
                Codec::Hevc => MFVideoFormat_HEVC,
                Codec::H264 => MFVideoFormat_H264,
            };
            wr(unsafe { mt.SetGUID(&MF_MT_SUBTYPE, &codec_guid) })?;
            Self::set_attribute_size(&mt, &MF_MT_FRAME_SIZE, width, height)?;
            Self::set_attribute_ratio(&mt, &MF_MT_FRAME_RATE, fps, 1)?;
            wr(unsafe { mt.SetUINT32(&MF_MT_AVG_BITRATE, bitrate) })?;
            wr(unsafe { mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32) })?;
            mt
        };

        let video_stream_index = wr(unsafe { sink_writer.AddStream(&video_output_type) })?;

        let video_input_type: IMFMediaType = {
            let mt = wr(unsafe { MFCreateMediaType() })?;
            wr(unsafe { mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) })?;
            wr(unsafe { mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32) })?;
            Self::set_attribute_size(&mt, &MF_MT_FRAME_SIZE, width, height)?;
            Self::set_attribute_ratio(&mt, &MF_MT_FRAME_RATE, fps, 1)?;
            wr(unsafe { mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32) })?;
            mt
        };

        wr(unsafe { sink_writer.SetInputMediaType(video_stream_index, &video_input_type, None) })?;

        let audio_stream_index = if let Some(audio) = audio_mt {
            Some(Self::add_audio_stream(&sink_writer, audio)?)
        } else {
            None
        };

        wr(unsafe { sink_writer.BeginWriting() })?;

        Ok(Self {
            sink_writer: Some(sink_writer),
            video_stream_index,
            audio_stream_index,
            width,
            height,
            fps,
            started: true,
            finalized: false,
            video_sample_count: 0,
            video_start_time: None,
            audio_time: 0,
        })
    }

    pub fn write_video(&mut self, frame: &super::dxgi::FrameData) -> Result<()> {
        let sink_writer = self.sink_writer.as_ref()
            .ok_or_else(|| anyhow!("encoder finalized"))?;

        let now = std::time::Instant::now();
        let elapsed = self.video_start_time.get_or_insert(now).elapsed();
        let sample_time = (elapsed.as_secs() as i64) * 10_000_000 + (elapsed.subsec_nanos() as i64) / 100;
        let sample_duration = 10_000_000 / self.fps as i64;

        let row_bytes = (frame.width * 4) as usize;
        let total = row_bytes * frame.height as usize;
        let mut bgra = Vec::with_capacity(total);
        for y in (0..frame.height as usize).rev() {
            let off = y * frame.pitch as usize;
            bgra.extend_from_slice(&frame.data[off..off + row_bytes]);
        }

        let sample = {
            let s = wr(unsafe { MFCreateSample() })?;
            let buffer = wr(unsafe { MFCreateMemoryBuffer(bgra.len() as u32) })?;
            unsafe {
                let mut byte_buffer: *mut u8 = ptr::null_mut();
                let mut max_len: u32 = 0;
                let mut cur_len: u32 = 0;
                wr(buffer.Lock(&mut byte_buffer, Some(&mut max_len), Some(&mut cur_len)))?;
                ptr::copy_nonoverlapping(bgra.as_ptr(), byte_buffer, bgra.len());
                wr(buffer.SetCurrentLength(bgra.len() as u32))?;
                wr(buffer.Unlock())?;
            }
            wr(unsafe { s.AddBuffer(&buffer) })?;
            wr(unsafe { s.SetSampleTime(sample_time) })?;
            wr(unsafe { s.SetSampleDuration(sample_duration) })?;
            s
        };

        wr(unsafe { sink_writer.WriteSample(self.video_stream_index, &sample) })?;
        self.video_sample_count += 1;
        Ok(())
    }

    pub fn write_audio(&mut self, samples: &super::audio::AudioSamples) -> Result<()> {
        let Some(audio_stream) = self.audio_stream_index else { return Ok(()) };
        let sink_writer = self.sink_writer.as_ref()
            .ok_or_else(|| anyhow!("encoder finalized"))?;

        let frame_size = (samples.bits_per_sample as u64 / 8) * samples.channels as u64;
        let sample_rate = samples.sample_rate;

        let sample_duration = if frame_size > 0 && sample_rate > 0 {
            (samples.data.len() as i64 * 10_000_000) / (frame_size as i64 * sample_rate as i64)
        } else {
            0i64
        };

        let sample = {
            let s = wr(unsafe { MFCreateSample() })?;
            let buffer = wr(unsafe { MFCreateMemoryBuffer(samples.data.len() as u32) })?;
            unsafe {
                let mut byte_buffer: *mut u8 = ptr::null_mut();
                let mut max_len: u32 = 0;
                let mut cur_len: u32 = 0;
                wr(buffer.Lock(&mut byte_buffer, Some(&mut max_len), Some(&mut cur_len)))?;
                ptr::copy_nonoverlapping(samples.data.as_ptr(), byte_buffer, samples.data.len());
                wr(buffer.SetCurrentLength(samples.data.len() as u32))?;
                wr(buffer.Unlock())?;
            }
            wr(unsafe { s.AddBuffer(&buffer) })?;
            wr(unsafe { s.SetSampleTime(self.audio_time) })?;
            wr(unsafe { s.SetSampleDuration(sample_duration) })?;
            s
        };

        wr(unsafe { sink_writer.WriteSample(audio_stream, &sample) })?;
        self.audio_time += sample_duration;
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<()> {
        if self.finalized { return Ok(()); }
        self.finalized = true;

        if let Some(sink_writer) = self.sink_writer.take() {
            wr(unsafe { sink_writer.Finalize() })?;
        }

        unsafe { MFShutdown().ok(); }
        Ok(())
    }

    fn create_sink_writer_attributes() -> Result<IMFAttributes> {
        unsafe {
            let mut attrs: Option<IMFAttributes> = None;
            wr(MFCreateAttributes(&mut attrs, 2))?;
            let attrs = attrs.ok_or_else(|| anyhow!("no MF attributes"))?;
            let _ = attrs.SetUINT32(&MF_READWRITE_DISABLE_CONVERTERS, 0);
            let _ = attrs.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1);
            Ok(attrs)
        }
    }

    fn add_audio_stream(sink_writer: &IMFSinkWriter, audio: &AudioMediaType) -> Result<u32> {
        let output_type: IMFMediaType = {
            let mt = wr(unsafe { MFCreateMediaType() })?;
            wr(unsafe { mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio) })?;
            wr(unsafe { mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC) })?;
            wr(unsafe { mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, audio.sample_rate) })?;
            wr(unsafe { mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, audio.channels as u32) })?;
            wr(unsafe { mt.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 16000) })?;
            mt
        };

        let stream_index = wr(unsafe { sink_writer.AddStream(&output_type) })?;

        let input_type: IMFMediaType = {
            let mt = wr(unsafe { MFCreateMediaType() })?;
            wr(unsafe { mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio) })?;
            wr(unsafe { mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM) })?;
            wr(unsafe { mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, audio.sample_rate) })?;
            wr(unsafe { mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, audio.channels as u32) })?;
            wr(unsafe { mt.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16u32) })?;
            let block_align = 2 * audio.channels as u32;
            wr(unsafe { mt.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_align) })?;
            let avg_bytes = audio.sample_rate * audio.channels as u32 * 2;
            wr(unsafe { mt.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, avg_bytes) })?;
            wr(unsafe { mt.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1) })?;
            mt
        };

        wr(unsafe { sink_writer.SetInputMediaType(stream_index, &input_type, None) })?;
        Ok(stream_index)
    }

    fn set_attribute_size(mt: &IMFMediaType, key: &GUID, w: u32, h: u32) -> Result<()> {
        let packed = ((w as u64) << 32) | (h as u64);
        wr(unsafe { mt.SetUINT64(key, packed) })
    }

    fn set_attribute_ratio(mt: &IMFMediaType, key: &GUID, num: u32, den: u32) -> Result<()> {
        let packed = ((num as u64) << 32) | (den as u64);
        wr(unsafe { mt.SetUINT64(key, packed) })
    }

    fn target_bitrate(width: u32, height: u32, fps: u32, quality: Quality, codec: Codec) -> u32 {
        let pixels_per_second = width as u64 * height as u64 * fps as u64;
        let bits_per_pixel = match quality {
            Quality::Efficient => 0.045,
            Quality::Balanced => 0.07,
            Quality::High => 0.105,
        };
        let codec_scale = match codec {
            Codec::Hevc => 1.0,
            Codec::H264 => 1.35,
        };
        (pixels_per_second as f64 * bits_per_pixel * codec_scale).max(1_500_000.0) as u32
    }
}

impl Drop for MfEncoder {
    fn drop(&mut self) {
        if self.started { let _ = self.finalize(); }
    }
}

#[allow(dead_code)]
pub fn bgra_to_nv12(bgra: &[u8], width: u32, height: u32, _pitch: u32) -> Vec<u8> {
    let y_plane_size = (width * height) as usize;
    let uv_plane_size = ((width / 2) * (height / 2) * 2) as usize;
    let mut nv12 = vec![0u8; y_plane_size + uv_plane_size];

    let (y_plane, uv_plane) = nv12.split_at_mut(y_plane_size);

    for y in 0..height {
        for x in 0..width {
            let src_idx = ((y * width + x) * 4) as usize;
            if src_idx + 3 >= bgra.len() { continue; }
            let b = bgra[src_idx] as i32;
            let g = bgra[src_idx + 1] as i32;
            let r = bgra[src_idx + 2] as i32;

            let y_val = ((66i32 * r + 129 * g + 25 * b + 128) >> 8) + 16;
            y_plane[(y * width + x) as usize] = y_val.clamp(0, 255) as u8;

            if y % 2 == 0 && x % 2 == 0 {
                let u_val = ((-38i32 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                let v_val = ((112i32 * r - 94 * g - 18 * b + 128) >> 8) + 128;
                let uv_idx = ((y / 2) * (width / 2) * 2 + (x / 2) * 2) as usize;
                if uv_idx + 1 < uv_plane.len() {
                    uv_plane[uv_idx] = u_val.clamp(0, 255) as u8;
                    uv_plane[uv_idx + 1] = v_val.clamp(0, 255) as u8;
                }
            }
        }
    }
    nv12
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_to_nv12_small() {
        let w = 4u32; let h = 4u32;
        let bgra = vec![128u8; (w * h * 4) as usize];
        let nv12 = bgra_to_nv12(&bgra, w, h, w * 4);
        assert_eq!(nv12.len(), (w * h + (w / 2) * (h / 2) * 2) as usize);
    }

    #[test]
    fn target_bitrate_minimum_is_1_5_mbps() {
        let rate = MfEncoder::target_bitrate(320, 240, 10, Quality::Efficient, Codec::Hevc);
        assert!(rate >= 1_500_000);
    }
}
