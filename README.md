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
  Windows port of <a href="https://github.com/shivamhwp/wrec">shivamhwp/wrec</a> — the most efficient screen recorder.
</p>

<p align="center">
  ⚠️ <b>WIP</b> — Raw DXGI capture, WASAPI audio, MF encoding. CLI works, GUI coming later.
</p>

<p align="center">
  <a href="https://github.com/shivamhwp/wrec" target="_blank" rel="noopener noreferrer">shivamhwp/wrec</a>
  &nbsp;·&nbsp;
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

Windows-native rewrite using DXGI output duplication, WASAPI loopback capture,
and Media Foundation H.264/HEVC encoding. No external dependencies — pure Rust
and Win32 APIs via the `windows` crate.

## Install

```powershell
# Build capture-engine
cargo build -p windows-recorder --bin capture-engine

# List display targets
.\target\debug\capture-engine.exe --list

# Record display 0 for 5 seconds (type "stop" to end)
.\target\debug\capture-engine.exe output.mp4 30 true display 0 h264 balanced native true true raw
```

Requires Rust 1.78+ and the `x86_64-pc-windows-msvc` target.

## License

MIT — same as the original [wrec](https://github.com/shivamhwp/wrec).
