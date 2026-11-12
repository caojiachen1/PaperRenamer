# PaperRenamer

Rename paper PDFs from their titles, fully offline.

- Double-click the exe for the GPUI GUI, or drag PDFs/folders onto it to rename copies in place.
- Titles come from embedded XMP plus a Rust port of GROBID's header CRF. Fully offline: no
  network access, no API keys.
- Pure-Rust GPUI desktop app, single self-contained exe: no WebView, no Java, no OCR.

## Build

Requirements: Rust (MSVC toolchain) and the Windows SDK `fxc.exe` (GPUI compiles its shaders
at build time).

```
copy .cargo\config.toml.example .cargo\config.toml
rem edit GPUI_FXC_PATH if your Windows SDK lives elsewhere
cargo build --release
```

The output is `target\release\PaperRenamer.exe`.

The previous Tauri/WebView version is preserved on the `legacy` branch.
