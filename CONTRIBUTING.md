# Contributing

Windows-only fork of [shivamhwp/wrec](https://github.com/shivamhwp/wrec).

## Prerequisites

- Windows 10/11
- Rust 1.78+ (`x86_64-pc-windows-msvc` toolchain)
- Visual Studio build tools (or VS 2022 with "Desktop development with C++")

## Build & run

```powershell
$env:CARGO_TARGET_DIR = "$env:TEMP\wrec-target"
cargo run --bin wrec -- daemon serve   # terminal 1
cargo run --bin wrec -- record         # terminal 2
```

## Code style

See `AGENTS.md` for design philosophy. Keep it simple, keep it native, measure
everything.
