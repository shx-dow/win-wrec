use windows::Win32::Media::MediaFoundation::*;
fn main() -> windows::core::Result<()> {
    unsafe {
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
        let input: IMFMediaType = MFCreateMediaType()?;
        let output: IMFMediaType = MFCreateMediaType()?;
        println!("IMFMediaType works");
        MFShutdown();
    }
    Ok(())
}
