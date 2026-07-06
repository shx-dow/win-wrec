use anyhow::{anyhow, bail, Context as AnyhowContext, Result};
use std::{
    ffi::c_void,
    io::BufRead,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::RPC_E_CHANGED_MODE,
    Media::{
        Audio::{
            eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
            MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVE_FORMAT_PCM,
        },
        KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE},
        Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT},
    },
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED,
    },
};
use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
        VideoSettingsSubType,
    },
    frame::Frame,
    graphics_capture_api::{GraphicsCaptureApi, InternalCaptureControl},
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings,
        Settings,
    },
    window::Window,
};

use crate::RecordArgs;

struct CaptureFlags {
    output_path: std::path::PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    bitrate: u32,
    codec: VideoSettingsSubType,
    include_system_audio: bool,
    paused: Arc<AtomicBool>,
}

struct Capture {
    encoder: Arc<Mutex<Option<VideoEncoder>>>,
    paused: Arc<AtomicBool>,
    audio_running: Option<Arc<AtomicBool>>,
    audio_thread: Option<thread::JoinHandle<()>>,
    width: u32,
    height: u32,
    packed_frame: Vec<u8>,
    scaled_frame: Vec<u8>,
    started_at: Instant,
    last_metric_at: Instant,
    frame_count: u64,
    dropped_frame_count: u64,
}

impl Capture {
    fn finish(&mut self) -> Result<()> {
        if let Some(running) = &self.audio_running {
            running.store(false, Ordering::Relaxed);
        }
        if let Some(audio_thread) = self.audio_thread.take() {
            if let Err(err) = audio_thread.join() {
                eprintln!("capture-engine: audio capture thread panicked: {err:?}");
            }
        }

        let encoder = self.encoder.lock().unwrap().take();
        if let Some(encoder) = encoder {
            encoder.finish()?;
        }
        eprintln!(
            "capture-engine: recording finished frames={} dropped={}",
            self.frame_count, self.dropped_frame_count
        );
        Ok(())
    }
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = CaptureFlags;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        let flags = ctx.flags;
        let audio_format = if flags.include_system_audio {
            Some(default_loopback_audio_format()?)
        } else {
            None
        };
        let audio_settings = audio_format
            .as_ref()
            .map(|format| {
                AudioSettingsBuilder::new()
                    .channel_count(format.channels as u32)
                    .sample_rate(format.sample_rate)
                    .bit_per_sample(16)
            })
            .unwrap_or_else(|| AudioSettingsBuilder::new().disabled(true));
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(flags.width, flags.height)
                .sub_type(flags.codec)
                .bitrate(flags.bitrate)
                .frame_rate(flags.fps),
            audio_settings,
            ContainerSettingsBuilder::new(),
            &flags.output_path,
        )?;
        let encoder = Arc::new(Mutex::new(Some(encoder)));
        let (audio_running, audio_thread) = audio_format
            .map(|format| {
                spawn_audio_capture_thread(format, encoder.clone(), flags.paused.clone())
            })
            .transpose()?
            .map_or((None, None), |(running, handle)| {
                (Some(running), Some(handle))
            });

        eprintln!("capture-engine: recording started");
        Ok(Self {
            encoder,
            paused: flags.paused,
            audio_running,
            audio_thread,
            width: flags.width,
            height: flags.height,
            packed_frame: Vec::new(),
            scaled_frame: Vec::new(),
            started_at: Instant::now(),
            last_metric_at: Instant::now(),
            frame_count: 0,
            dropped_frame_count: 0,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        if self.paused.load(Ordering::Relaxed) {
            self.dropped_frame_count += 1;
            return Ok(());
        }

        let sent = {
            let mut encoder = self.encoder.lock().unwrap();
            if let Some(encoder) = encoder.as_mut() {
                if frame.width() == self.width && frame.height() == self.height {
                    encoder.send_frame(frame)?;
                } else {
                    send_scaled_frame(
                        frame,
                        encoder,
                        self.width,
                        self.height,
                        &mut self.packed_frame,
                        &mut self.scaled_frame,
                    )?;
                }
                true
            } else {
                false
            }
        };

        if sent {
            self.frame_count += 1;
            self.emit_metrics_if_needed();
        } else {
            self.dropped_frame_count += 1;
        }

        Ok(())
    }

    fn on_closed(&mut self) -> std::result::Result<(), Self::Error> {
        eprintln!("capture-engine: capture target closed");
        Ok(())
    }
}

impl Capture {
    fn emit_metrics_if_needed(&mut self) {
        if self.last_metric_at.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_metric_at = Instant::now();
        eprintln!(
            "capture-engine: metrics elapsed={} frames={} dropped={}",
            self.started_at.elapsed().as_secs(),
            self.frame_count,
            self.dropped_frame_count
        );
    }
}

enum CaptureCommand {
    Pause,
    Resume,
    Stop,
}

pub fn is_capture_supported() -> Result<bool> {
    GraphicsCaptureApi::is_supported().map_err(|e| anyhow!("{e}"))
}

pub fn list_targets() -> Result<()> {
    for (idx, monitor) in
        Monitor::enumerate()
            .context("failed to enumerate displays")?
            .iter()
            .enumerate()
    {
        let id = idx as u64;
        let name = monitor
            .name()
            .or_else(|_| monitor.device_string())
            .or_else(|_| monitor.device_name())
            .unwrap_or_else(|_| format!("Display {idx}"));
        println!("display\t{id}\t{}", sanitize_target_name(&name));
    }

    let current_pid = std::process::id();
    for window in Window::enumerate().context("failed to enumerate windows")? {
        if !window.is_valid() || window.process_id().ok() == Some(current_pid) {
            continue;
        }
        let title = window.title().unwrap_or_else(|_| "Window".to_string());
        if title.trim().is_empty() {
            continue;
        }
        let process = window.process_name().unwrap_or_else(|_| "App".to_string());
        let id = window.as_raw_hwnd() as usize as u64;
        println!(
            "window\t{id}\t{}",
            sanitize_target_name(&format!("{process} - {title}"))
        );
    }

    Ok(())
}

fn sanitize_target_name(name: &str) -> String {
    name.replace('\t', " ")
        .replace('\r', " ")
        .replace('\n', " ")
}

pub fn start_recording(args: RecordArgs) -> Result<()> {
    match args.target_kind.as_str() {
        "window" => {
            let window = Window::from_raw_hwnd(handle_from_id(args.target_id));
            if !window.is_valid() {
                bail!("window not found or is not capturable");
            }
            let native_width = window.width().context("failed to get window width")?;
            let native_height = window.height().context("failed to get window height")?;
            let size = encoder_size(
                native_width.max(2) as u32,
                native_height.max(2) as u32,
                &args.resolution,
            );
            run_capture(window, args, size)
        }
        "display" => {
            let monitors = Monitor::enumerate().context("failed to enumerate displays")?;
            let count = monitors.len();
            let monitor = monitors
                .into_iter()
                .nth(args.target_id as usize)
                .ok_or_else(|| {
                    if count == 0 {
                        anyhow!("no displays available")
                    } else if count == 1 {
                        anyhow!(
                            "display {} not found (only 1 display available, index 0)",
                            args.target_id
                        )
                    } else {
                        anyhow!(
                            "display {} not found (only {} displays available, indices 0-{})",
                            args.target_id,
                            count,
                            count - 1
                        )
                    }
                })?;
            let native_width = monitor.width().context("failed to get display width")?;
            let native_height = monitor.height().context("failed to get display height")?;
            let size = encoder_size(native_width, native_height, &args.resolution);
            run_capture(monitor, args, size)
        }
        other => bail!("unknown target kind `{other}`"),
    }
}

fn run_capture<T>(item: T, args: RecordArgs, (width, height): (u32, u32)) -> Result<()>
where
    T: TryInto<GraphicsCaptureItemType> + Send + 'static,
{
    let paused = Arc::new(AtomicBool::new(false));
    let flags = CaptureFlags {
        output_path: args.output_path.clone(),
        width,
        height,
        fps: args.fps,
        bitrate: target_bitrate(width, height, args.fps, &args.quality, &args.codec),
        codec: match args.codec.as_str() {
            "h264" => VideoSettingsSubType::H264,
            _ => VideoSettingsSubType::HEVC,
        },
        include_system_audio: args.include_system_audio,
        paused: paused.clone(),
    };

    let settings = Settings::new(
        item,
        if args.include_cursor {
            CursorCaptureSettings::WithCursor
        } else {
            CursorCaptureSettings::WithoutCursor
        },
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Include,
        MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(1.0 / args.fps as f64)),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    );
    let control = Capture::start_free_threaded(settings)
        .map_err(|err| anyhow!("failed to start Windows Graphics Capture: {err}"))?;
    let callback = control.callback();
    let commands = spawn_command_reader();

    let capture_result = loop {
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
                break control.stop();
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }

        if control.is_finished() {
            break control.wait();
        }

        thread::sleep(Duration::from_millis(50));
    };

    let finish_result = callback.lock().finish();
    capture_result.map_err(|err| anyhow!("capture thread failed: {err}"))?;
    finish_result?;
    Ok(())
}

fn spawn_command_reader() -> mpsc::Receiver<CaptureCommand> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines().map_while(std::result::Result::ok) {
            let command = match line.trim().to_lowercase().as_str() {
                "pause" => CaptureCommand::Pause,
                "resume" => CaptureCommand::Resume,
                "stop" => CaptureCommand::Stop,
                _ => continue,
            };
            if tx.send(command).is_err() {
                return;
            }
        }
        let _ = tx.send(CaptureCommand::Stop);
    });
    rx
}

#[derive(Clone, Copy)]
struct LoopbackAudioFormat {
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    block_align: u16,
    sample_kind: LoopbackSampleKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoopbackSampleKind {
    Float,
    Pcm,
}

fn default_loopback_audio_format() -> Result<LoopbackAudioFormat> {
    unsafe {
        let should_uninitialize = co_initialize_mta()?;
        let format = query_default_loopback_format();
        if should_uninitialize {
            CoUninitialize();
        }
        format
    }
}

unsafe fn co_initialize_mta() -> Result<bool> {
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr.is_ok() {
        Ok(true)
    } else if hr == RPC_E_CHANGED_MODE {
        Ok(false)
    } else {
        hr.ok()?;
        Ok(false)
    }
}

fn spawn_audio_capture_thread(
    format: LoopbackAudioFormat,
    encoder: Arc<Mutex<Option<VideoEncoder>>>,
    paused: Arc<AtomicBool>,
) -> Result<(Arc<AtomicBool>, thread::JoinHandle<()>)> {
    let running = Arc::new(AtomicBool::new(true));
    let thread_running = running.clone();
    let (startup_tx, startup_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        if let Err(err) =
            run_loopback_audio(format, encoder, paused, thread_running, startup_tx)
        {
            eprintln!("capture-engine: audio capture failed: {err}");
        }
    });

    match startup_rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok((running, handle)),
        Ok(Err(message)) => {
            running.store(false, Ordering::Relaxed);
            let _ = handle.join();
            Err(anyhow!(message))
        }
        Err(err) => {
            running.store(false, Ordering::Relaxed);
            let _ = handle.join();
            Err(anyhow!("audio capture did not start within 3s: {err}"))
        }
    }
}

fn run_loopback_audio(
    format: LoopbackAudioFormat,
    encoder: Arc<Mutex<Option<VideoEncoder>>>,
    paused: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    startup: mpsc::Sender<std::result::Result<(), String>>,
) -> Result<()> {
    let mut startup = Some(startup);
    let result = unsafe {
        let should_uninitialize = co_initialize_mta()?;
        let result = capture_loopback_audio(format, encoder, paused, running, &mut startup);
        if should_uninitialize {
            CoUninitialize();
        }
        result
    };

    if let Some(startup) = startup.take() {
        let _ = startup.send(result.as_ref().map(|_| ()).map_err(ToString::to_string));
    }
    result
}

unsafe fn capture_loopback_audio(
    format: LoopbackAudioFormat,
    encoder: Arc<Mutex<Option<VideoEncoder>>>,
    paused: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    startup: &mut Option<mpsc::Sender<std::result::Result<(), String>>>,
) -> Result<()> {
    let (audio_client, capture_client) = unsafe { open_loopback_capture_client()? };
    unsafe { audio_client.Start()? };
    if let Some(startup) = startup.take() {
        let _ = startup.send(Ok(()));
    }
    eprintln!(
        "capture-engine: system audio started sample_rate={} channels={} source_bits={}",
        format.sample_rate, format.channels, format.bits_per_sample
    );

    while running.load(Ordering::Relaxed) {
        let mut packet_size = unsafe { capture_client.GetNextPacketSize()? };
        while packet_size > 0 {
            let mut data = std::ptr::null_mut::<u8>();
            let mut frames = 0_u32;
            let mut flags = 0_u32;
            unsafe {
                capture_client.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?
            };

            let audio = unsafe { convert_loopback_buffer(data, frames, flags, format) };
            unsafe { capture_client.ReleaseBuffer(frames)? };

            if !paused.load(Ordering::Relaxed) && !audio.is_empty() {
                let mut encoder = encoder.lock().unwrap();
                if let Some(encoder) = encoder.as_mut() {
                    if let Err(err) = encoder.send_audio_buffer(&audio, 0) {
                        eprintln!("capture-engine: failed to send audio buffer: {err}");
                        running.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            }

            packet_size = unsafe { capture_client.GetNextPacketSize()? };
        }
        thread::sleep(Duration::from_millis(10));
    }

    let _ = unsafe { audio_client.Stop() };
    Ok(())
}

unsafe fn open_loopback_capture_client() -> Result<(IAudioClient, IAudioCaptureClient)> {
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? };
    let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    let mix_format = unsafe { audio_client.GetMixFormat()? };
    let initialize_result = unsafe {
        audio_client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            10_000_000,
            0,
            mix_format,
            None,
        )
    };
    unsafe { CoTaskMemFree(Some(mix_format.cast())) };
    initialize_result?;
    let capture_client = unsafe { audio_client.GetService::<IAudioCaptureClient>()? };
    Ok((audio_client, capture_client))
}

unsafe fn query_default_loopback_format() -> Result<LoopbackAudioFormat> {
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? };
    let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    let mix_format = unsafe { audio_client.GetMixFormat()? };
    let format = unsafe { parse_wave_format(mix_format) };
    unsafe { CoTaskMemFree(Some(mix_format.cast())) };
    format
}

unsafe fn parse_wave_format(format: *const WAVEFORMATEX) -> Result<LoopbackAudioFormat> {
    let wave = unsafe { std::ptr::read_unaligned(format) };
    let format_tag = wave.wFormatTag as u32;
    let channels = wave.nChannels;
    let sample_rate = wave.nSamplesPerSec;
    let bits_per_sample = wave.wBitsPerSample;
    let block_align = wave.nBlockAlign;
    let sample_kind = match format_tag {
        WAVE_FORMAT_PCM => LoopbackSampleKind::Pcm,
        WAVE_FORMAT_IEEE_FLOAT => LoopbackSampleKind::Float,
        WAVE_FORMAT_EXTENSIBLE => {
            let extensible =
                unsafe { std::ptr::read_unaligned(format.cast::<WAVEFORMATEXTENSIBLE>()) };
            let sub_format =
                unsafe { std::ptr::addr_of!(extensible.SubFormat).read_unaligned() };
            if sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
                LoopbackSampleKind::Float
            } else if sub_format == KSDATAFORMAT_SUBTYPE_PCM {
                LoopbackSampleKind::Pcm
            } else {
                bail!("unsupported WASAPI loopback subtype {:?}", sub_format);
            }
        }
        tag => bail!("unsupported WASAPI loopback format tag {tag}"),
    };

    if !matches!(bits_per_sample, 16 | 24 | 32) {
        bail!("unsupported WASAPI loopback bit depth {}", bits_per_sample);
    }

    Ok(LoopbackAudioFormat {
        channels,
        sample_rate,
        bits_per_sample,
        block_align,
        sample_kind,
    })
}

unsafe fn convert_loopback_buffer(
    data: *const u8,
    frames: u32,
    flags: u32,
    format: LoopbackAudioFormat,
) -> Vec<u8> {
    let input_len = frames as usize * format.block_align as usize;
    let samples = frames as usize * format.channels as usize;
    let mut out = Vec::with_capacity(samples * 2);

    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
        out.resize(samples * 2, 0);
        return out;
    }

    match (format.sample_kind, format.bits_per_sample) {
        (LoopbackSampleKind::Float, 32) => {
            let input = unsafe { std::slice::from_raw_parts(data.cast::<f32>(), samples) };
            for sample in input {
                let sample = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
                out.extend_from_slice(&sample.to_le_bytes());
            }
        }
        (LoopbackSampleKind::Pcm, 16) => {
            let input = unsafe { std::slice::from_raw_parts(data, input_len) };
            out.extend_from_slice(input);
        }
        (LoopbackSampleKind::Pcm, 24) => {
            let input = unsafe { std::slice::from_raw_parts(data, input_len) };
            for chunk in input.chunks_exact(3) {
                let value = i32::from_le_bytes([
                    chunk[0],
                    chunk[1],
                    chunk[2],
                    if chunk[2] & 0x80 == 0 { 0 } else { 0xff },
                ]);
                out.extend_from_slice(&((value >> 8) as i16).to_le_bytes());
            }
        }
        (LoopbackSampleKind::Pcm, 32) => {
            let input = unsafe { std::slice::from_raw_parts(data.cast::<i32>(), samples) };
            for sample in input {
                out.extend_from_slice(&((sample >> 16) as i16).to_le_bytes());
            }
        }
        _ => out.resize(samples * 2, 0),
    }

    out
}

fn handle_from_id(id: u64) -> *mut c_void {
    (id as usize) as *mut c_void
}

fn even_dimension(value: u32) -> u32 {
    let value = value.max(2);
    value - (value % 2)
}

fn encoder_size(native_width: u32, native_height: u32, resolution: &str) -> (u32, u32) {
    let max_size = match resolution {
        "720p" => Some((1280, 720)),
        "1080p" => Some((1920, 1080)),
        "2k" => Some((2560, 1440)),
        "4k" => Some((3840, 2160)),
        _ => None,
    };

    let Some((max_width, max_height)) = max_size else {
        return (even_dimension(native_width), even_dimension(native_height));
    };

    let scale = (max_width as f64 / native_width as f64)
        .min(max_height as f64 / native_height as f64)
        .min(1.0);

    (
        even_dimension((native_width as f64 * scale).round() as u32),
        even_dimension((native_height as f64 * scale).round() as u32),
    )
}

fn send_scaled_frame(
    frame: &mut Frame,
    encoder: &mut VideoEncoder,
    target_width: u32,
    target_height: u32,
    packed_frame: &mut Vec<u8>,
    scaled_frame: &mut Vec<u8>,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let timestamp = frame.timestamp()?.Duration;
    let source_width = frame.width();
    let source_height = frame.height();
    let frame_buffer = frame.buffer()?;
    let source = frame_buffer.as_nopadding_buffer(packed_frame);

    scale_bgra_nearest(
        source,
        source_width,
        source_height,
        target_width,
        target_height,
        scaled_frame,
    );
    encoder.send_frame_buffer(scaled_frame, timestamp)?;
    Ok(())
}

fn scale_bgra_nearest(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
    output: &mut Vec<u8>,
) {
    let target_len = target_width as usize * target_height as usize * 4;
    output.resize(target_len, 0);

    for target_y in 0..target_height {
        let source_y = target_y as usize * source_height as usize / target_height as usize;
        let source_row = source_y * source_width as usize * 4;
        let output_row = target_y as usize * target_width as usize * 4;
        for target_x in 0..target_width {
            let source_x = target_x as usize * source_width as usize / target_width as usize;
            let source_index = source_row + source_x * 4;
            let output_index = output_row + target_x as usize * 4;
            output[output_index..output_index + 4]
                .copy_from_slice(&source[source_index..source_index + 4]);
        }
    }
}

fn target_bitrate(width: u32, height: u32, fps: u32, quality: &str, codec: &str) -> u32 {
    let pixels_per_second = width as f64 * height as f64 * fps as f64;
    let bits_per_pixel = match quality {
        "efficient" => 0.045,
        "high" => 0.105,
        _ => 0.07,
    };
    let codec_scale = if codec == "h264" { 1.35 } else { 1.0 };
    (pixels_per_second * bits_per_pixel * codec_scale)
        .round()
        .clamp(1_500_000.0, u32::MAX as f64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_size_scales_down_to_requested_maximum() {
        assert_eq!(encoder_size(3840, 2160, "1080p"), (1920, 1080));
        assert_eq!(encoder_size(3025, 1965, "720p"), (1108, 720));
        assert_eq!(encoder_size(1280, 720, "4k"), (1280, 720));
        assert_eq!(encoder_size(3025, 1965, "native"), (3024, 1964));
    }

    #[test]
    fn scale_bgra_nearest_picks_source_pixels_by_ratio() {
        let source = [
            1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255,
            7, 0, 0, 255, 8, 0, 0, 255, 9, 0, 0, 255, 10, 0, 0, 255, 11, 0, 0, 255, 12, 0, 0,
            255, 13, 0, 0, 255, 14, 0, 0, 255, 15, 0, 0, 255, 16, 0, 0, 255,
        ];
        let mut output = Vec::new();

        scale_bgra_nearest(&source, 4, 4, 2, 2, &mut output);

        assert_eq!(
            output,
            vec![1, 0, 0, 255, 3, 0, 0, 255, 9, 0, 0, 255, 11, 0, 0, 255]
        );
    }

    #[test]
    fn efficient_bitrate_is_lower_than_high() {
        assert!(
            target_bitrate(1920, 1080, 30, "efficient", "hevc")
                < target_bitrate(1920, 1080, 30, "high", "hevc")
        );
    }
}
