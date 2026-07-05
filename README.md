<p align="center">
  <img src="images/wrec.png" alt="wrec" width="112" />
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="images/wrec-title-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="images/wrec-title-light.svg">
    <img src="images/wrec-title-light.svg" alt="wrec" width="92" />
  </picture>
</p>

<p align="center">
  Windows fork of <a href="https://github.com/shivamhwp/wrec">shivamhwp/wrec</a> — the most efficient screen recorder.
</p>

<p align="center">
  ⚠️ <b>WIP</b> — DXGI capture, WASAPI audio, MF encoding. CLI works, GUI coming later.
</p>

<p align="center">
  <a href="https://github.com/shivamhwp/wrec" target="_blank" rel="noopener noreferrer">Original wrec</a>
  &nbsp;·&nbsp;
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

Windows-native rewrite using DXGI output duplication, WASAPI loopback capture,
and Media Foundation H.264/HEVC encoding. No external dependencies — pure Rust
and Win32 APIs via the `windows` crate.

## Install

```powershell
# Build release binary
cargo build --release -p cli -p daemon

# Copy to a PATH directory (example: User Programs)
mkdir -Force "$env:LOCALAPPDATA\Programs\winwrec\bin" | Out-Null
copy target\release\winwrec.exe "$env:LOCALAPPDATA\Programs\winwrec\bin\"
copy target\release\daemon.exe "$env:LOCALAPPDATA\Programs\winwrec\bin\"

# Add to PATH (run once, restart terminal)
[Environment]::SetEnvironmentVariable(
  "PATH",
  [Environment]::GetEnvironmentVariable("PATH", "User") + ";$env:LOCALAPPDATA\Programs\winwrec\bin",
  "User"
)
```

Requires Rust 1.78+ and the `x86_64-pc-windows-msvc` target (installed by default
with rustup on Windows).

## Quick start

```powershell
# Start the daemon
winwrec daemon serve

# List capture targets
winwrec targets --json

# Record for 10 seconds
winwrec record --duration 10

# Or record until Ctrl+C
winwrec record
```

Output is written to `~\Videos\Wrec\wrec-<unix-ts>.mp4`.

## License

MIT — same as the original [wrec](https://github.com/shivamhwp/wrec).
