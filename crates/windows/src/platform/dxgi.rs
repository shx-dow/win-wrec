use domain::{CaptureSourceKind, CaptureTarget, RecorderError, Result};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Direct3D::{D3D_FEATURE_LEVEL, D3D_DRIVER_TYPE};
use windows::Win32::System::Com::*;
use windows::Win32::Foundation::HMODULE;
use windows::core::Interface;

fn wr<T>(r: windows::core::Result<T>) -> Result<T> {
    r.map_err(|e| RecorderError::Backend(e.to_string()))
}

pub struct DxgiCapture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    staging_texture: Option<ID3D11Texture2D>,
    width: u32,
    height: u32,
}

#[derive(Clone)]
pub struct FrameData {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}

const FRAME_TIMEOUT: u32 = 100;

pub fn enumerate_targets() -> Result<Vec<CaptureTarget>> {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok(); }
    let factory: IDXGIFactory1 = wr(unsafe { CreateDXGIFactory1() })?;
    let mut targets = Vec::new();
    let mut ai = 0u32;
    loop {
        let Ok(adapter) = (unsafe { factory.EnumAdapters(ai) }) else { break };
        let desc = match unsafe { adapter.GetDesc() } {
            Ok(d) => d,
            Err(_) => { ai += 1; continue; }
        };
        let aname = String::from_utf16_lossy(&desc.Description).trim_end_matches('\0').to_string();
        let mut oi = 0u32;
        loop {
            if unsafe { adapter.EnumOutputs(oi) }.is_err() { break; }
            targets.push(CaptureTarget {
                id: oi as u64,
                name: format!("Display {} ({})", oi + 1, aname),
                kind: CaptureSourceKind::Display,
            });
            oi += 1;
        }
        ai += 1;
    }
    Ok(targets)
}

impl DxgiCapture {
    pub fn new(target: &CaptureTarget) -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok(); }
        let factory: IDXGIFactory1 = wr(unsafe { CreateDXGIFactory1() })?;

        let mut found_output: Option<IDXGIOutput> = None;
        let mut found_adapter: Option<IDXGIAdapter> = None;
        let ti = target.id as u32;
        let mut ai = 0u32;
        'outer: loop {
        let Ok(adapter) = (unsafe { factory.EnumAdapters(ai) }) else { break };
            for oi in 0u32.. {
                let Ok(output) = (unsafe { adapter.EnumOutputs(oi) }) else { break };
                if oi == ti { found_output = Some(output); found_adapter = Some(adapter); break 'outer; }
            }
            ai += 1;
        }
        if found_output.is_none() && ai == 0 && ti == 0 {
            let adapter = wr(unsafe { factory.EnumAdapters(0) })?;
            let output = wr(unsafe { adapter.EnumOutputs(0) })?;
            found_adapter = Some(adapter);
            found_output = Some(output);
        }

        let output = found_output.ok_or_else(|| RecorderError::Backend(format!("display {} not found", target.id)))?;
        let adapter = found_adapter.unwrap();

        let output1: IDXGIOutput1 = output.cast()
            .map_err(|e| RecorderError::Backend(format!("IDXGIOutput1: {e}")))?;

        let (device, context) = Self::create_device(&adapter)?;
        let duplication = wr(unsafe { output1.DuplicateOutput(&device) }).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("0x887a0004") || msg.contains("0x887a0027") {
                RecorderError::MissingScreenRecordingPermission
            } else { e }
        })?;

        let dup_desc = unsafe { duplication.GetDesc() };
        let w = dup_desc.ModeDesc.Width.max(1);
        let h = dup_desc.ModeDesc.Height.max(1);

        Ok(Self { device, context, duplication, staging_texture: None, width: w, height: h })
    }

    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }

    pub fn media_type(&self) -> super::encoder::VideoMediaType {
        super::encoder::VideoMediaType { width: self.width, height: self.height, frame_rate_numerator: 30, frame_rate_denominator: 1 }
    }

    pub fn acquire_frame(&mut self) -> Result<FrameData> {
        unsafe {
            let mut resource: Option<IDXGIResource> = None;
            let mut fi = DXGI_OUTDUPL_FRAME_INFO::default();
            let hr = self.duplication.AcquireNextFrame(FRAME_TIMEOUT, &mut fi, &mut resource as *mut Option<IDXGIResource>);
            if let Err(e) = &hr {
                let code = e.code();
                if code == windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAIT_TIMEOUT {
                    return Err(RecorderError::Backend("timeout".into()));
                }
                return Err(RecorderError::Backend(format!("AcquireNextFrame: {e}")));
            }
            if fi.LastPresentTime == 0 { let _ = self.duplication.ReleaseFrame(); return Err(RecorderError::Backend("timeout".into())); }
            let Some(res) = resource else { let _ = self.duplication.ReleaseFrame(); return Err(RecorderError::Backend("no resource".into())); };

            let src: ID3D11Texture2D = res.cast()
                .map_err(|e| { let _ = self.duplication.ReleaseFrame(); RecorderError::Backend(format!("texture: {e}")) })?;
            let mut sd = D3D11_TEXTURE2D_DESC::default();
            src.GetDesc(&mut sd);
            let need_new = self.staging_texture.is_none()
                || self.staging_texture.as_ref().map(|t| { let mut d = D3D11_TEXTURE2D_DESC::default(); t.GetDesc(&mut d); d.Width != sd.Width || d.Height != sd.Height }).unwrap_or(true);

            if need_new {
                let stg = D3D11_TEXTURE2D_DESC {
                    Width: sd.Width, Height: sd.Height, MipLevels: 1, ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_STAGING, BindFlags: 0, CPUAccessFlags: 0x20000, MiscFlags: 0,
                };
                let mut tex: Option<ID3D11Texture2D> = None;
                wr(self.device.CreateTexture2D(&stg, None, Some(&mut tex)))?;
                self.staging_texture = tex;
                self.width = sd.Width; self.height = sd.Height;
            }

            let stg = self.staging_texture.as_ref().unwrap();
            self.context.CopyResource(stg, &src);
            let _ = self.duplication.ReleaseFrame();
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            wr(self.context.Map(stg, 0, D3D11_MAP_READ, 0, Some(&mut mapped)))?;
            let pitch = mapped.RowPitch;
            let total = (pitch * self.height) as usize;
            let data = std::slice::from_raw_parts(mapped.pData as *const u8, total).to_vec();
            self.context.Unmap(stg, 0);
            Ok(FrameData { data, width: self.width, height: self.height, pitch })
        }
    }

    pub fn release_frame(&self) {}
    pub fn is_timeout(&self, err: &RecorderError) -> bool {
        matches!(err, RecorderError::Backend(m) if m == "timeout")
    }

    fn create_device(adapter: &IDXGIAdapter) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
        let fl = [D3D_FEATURE_LEVEL(0x0000b000), D3D_FEATURE_LEVEL(0x0000a100),
                  D3D_FEATURE_LEVEL(0x0000a000), D3D_FEATURE_LEVEL(0x00009200)];
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        wr(unsafe {
            D3D11CreateDevice(
                Some(adapter),
                D3D_DRIVER_TYPE(0),
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&fl[..]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        })?;
        let d = device.ok_or_else(|| RecorderError::Backend("no device".into()))?;
        let c = context.ok_or_else(|| RecorderError::Backend("no context".into()))?;
        Ok((d, c))
    }
}

impl Drop for DxgiCapture {
    fn drop(&mut self) { let _ = unsafe { self.duplication.ReleaseFrame() }; }
}
