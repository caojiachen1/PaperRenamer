# PaperRenamer

Rename paper PDFs from their titles, fully offline.

- Double-click the exe for the GUI, or drag PDFs/folders onto it to rename copies in place.
- Titles come from embedded XMP plus a Rust port of GROBID's header CRF; the optional CrossRef
  cross-check is off by default and remembered once enabled in the GUI (drag-and-drop batches follow it too).
- Single self-contained exe: no Java, no OCR, no side files.

## Build

```
pnpm install
pnpm tauri build --no-bundle
```
