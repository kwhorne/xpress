# xpress

[![CI](https://github.com/kwhorne/xpress/actions/workflows/ci.yml/badge.svg)](https://github.com/kwhorne/xpress/actions/workflows/ci.yml)
[![Release](https://github.com/kwhorne/xpress/actions/workflows/release.yml/badge.svg)](https://github.com/kwhorne/xpress/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

An **image, video, PDF and audio optimiser** written in Rust. Copy large, paste
small, send fast.

xpress is UI-agnostic: the core crate resolves external tools (`ffmpeg`,
`pngquant`, `jpegoptim`, `gifsicle`, `ghostscript`, `vips`, `gifski`, `cwebp`,
`heif-enc`) and drives them with a single, percentage-based compression model.

> xpress is an independent project under the [MIT License](LICENSE). It is
> **not** a fork of and contains no source from [Clop](https://lowtechguys.com/clop);
> it is inspired by Clop's functionality and ideas. See [NOTICE.md](NOTICE.md)
> for details and for the licences of any bundled third-party binaries.

## Architecture

```
xpress/
  crates/
    xpress-core/   # the optimisation engine (no UI)
      compression  # CompressionQuality model — exact port of Shared.swift formulas
      tools        # external-binary resolution + process execution
      filetype     # media-kind classification (image/video/audio/pdf)
      image        # jpeg (jpegoptim), png (pngquant), gif (gifsicle)
      video        # ffmpeg H.264 path
      pdf          # ghostscript
      audio        # ffmpeg (aac/mp3/opus/wav/flac/aiff) + AudioFormat
      result       # OptimisationResult, backup/dates/size helpers
    xpress-cli/    # `xpress` binary — the command-line interface
    xpress-gui/    # `xpress-gui` binary — desktop app (egui/eframe)
```

## Desktop app

```sh
cargo run -p xpress-gui --release
```

* **Drag and drop** images, videos, PDFs or audio onto the window to optimise them.
* **Result cards** show the before/after size, the saving, and a thumbnail.
* **⌘⇧O / Ctrl⇧O** (global hotkey) optimises the image currently on the clipboard.
* A **compression** slider, *aggressive* / *backup* / *strip metadata* toggles, an
  inline **pipeline** field, and a **float on top** option.
* Work runs off the UI thread, so the window stays responsive.

### Build a macOS `.app`

```sh
cargo build --release -p xpress-gui -p xpress-cli
scripts/make-app.sh                 # -> dist/xpress.app (ad-hoc signed)
scripts/make-app.sh --tools         # also bundle ffmpeg/pngquant/... into the app
```

The app is ad-hoc signed so it runs locally. Tagged releases also publish a
signed-for-local-use `xpress-*-app.zip`. For public distribution, sign with a
Developer ID and notarise (commands are documented at the bottom of
`scripts/make-app.sh`).

The app icon lives at `assets/AppIcon.icns`. To regenerate it from the vector
source after editing `assets/icon.svg`:

```sh
cargo run --manifest-path tools/icon-gen/Cargo.toml --release -- assets/icon.svg assets/xpress.iconset
iconutil -c icns assets/xpress.iconset -o assets/AppIcon.icns
```

## Optimisation tools

xpress shells out to external binaries. Resolution order:
1. `$XPRESS_BIN_DIR/<tool>`
2. a `bin/` directory next to the `xpress` executable
3. the per-user bundle dir (`~/Library/Application Support/xpress/bin`, or the
   XDG/LOCALAPPDATA equivalent)
4. the system `PATH`

Three ways to provide them:

```sh
# A) Install with a package manager and let PATH resolution find them
brew install ffmpeg pngquant jpegoptim gifsicle ghostscript vips gifski webp exiftool

# B) Bundle copies into the per-user dir (self-contained, no PATH dependency)
scripts/fetch-tools.sh

# C) Embed them inside the xpress executable (single self-contained file)
scripts/fetch-tools.sh --vendor       # populates vendor/bin/<target> + current/
cargo build --release --features embed-tools

xpress doctor    # show what is available
xpress bundle    # extract embedded binaries to the per-user dir
```

The bundled binaries keep their own upstream licences — see [NOTICE.md](NOTICE.md)
before redistributing xpress together with them.

## Usage

```sh
# Optimise anything (auto-detects type)
xpress optimise photo.png screencast.mov document.pdf

# A whole folder, recursively, with the aggressive preset
xpress optimise -r --aggressive ~/Screenshots

# Fine-grained compression (5 = best quality .. 100 = smallest)
xpress optimise --compression 64 photo.jpg

# Restrict to one media kind, downsample PDFs to 144 dpi
xpress optimise --kind pdf --pdf-dpi 144 *.pdf

# Convert audio
xpress convert --to mp3 --bitrate 192 recording.wav

# Crop to a size, an aspect ratio, or a long edge
xpress crop --size 1200x630 banner.png
xpress crop --size 16:9 --smart-crop photo.jpg
xpress crop --size 1920 --long-edge shot.png

# Pipelines: chain steps with `->`
xpress pipeline run 'crop(width: 1600) -> convert(to: webp) -> downscale(factor: 0.5)' photo.png
xpress pipeline add web 'crop(width: 1600) -> convert(to: webp)'
xpress pipeline run web *.png
xpress pipeline list

# Watch folders (and the clipboard) and optimise automatically
xpress pipeline attach ~/Screenshots 'crop(longEdge: 2000) -> convert(to: webp)' --type image
xpress watch                     # uses the saved automations
xpress watch --clipboard ~/Inbox # watch a folder + the clipboard

# Strip metadata
xpress strip-exif *.jpg
```

### Pipeline DSL

Steps are joined with `->` and run left-to-right, each feeding the next:

| Step | Example |
|------|---------|
| `optimise` | `optimise` |
| `downscale(factor:)` | `downscale(factor: 0.5)` or `downscale(factor: 50%)` |
| `crop(width:, height:, longEdge:, ratio:, smart:)` | `crop(width: 1600)`, `crop(ratio: 16:9)` |
| `convert(to:)` | `convert(to: webp)` (image) / `convert(to: mp3)` (audio) |
| `stripExif` | `stripExif` |
| `removeAudio` | `removeAudio` (video) |
| `changeSpeed(factor:)` | `changeSpeed(factor: 2.0)` |
| `capFps(fps:)` | `capFps(fps: 30)` |
| `lowerBitrate(kbps:)` | `lowerBitrate(kbps: 128)` (audio) |

Originals are backed up next to the file as `.<name>.orig` unless `--no-backup`.

## Status & roadmap

**Phase 1 — core engine + CLI** ✅
- Percentage-based compression model with unit tests on the preset anchors.
- Image (jpeg/png/gif), video (H.264), PDF (ghostscript), audio optimise/convert.
- Parallel batch optimisation, backups, size guard, metadata, timestamps.
- Self-contained binary bundling (`embed-tools`, `fetch-tools.sh`, `xpress bundle`).

**Phase 2 — scaling & conversion** ✅
- `downscale --factor` for images (vips/ffmpeg) and videos (ffmpeg `scale=`).
- Image format conversion (webp/avif/heic/jxl/png/jpeg) via cwebp/heif-enc/cjxl.
- Audio conversion (aac/mp3/opus/wav/flac/aiff).

**Phase 3 — crop & pipelines** ✅
- `crop` to a size, aspect ratio or long edge (vips smart crop / ffmpeg).
- Pipeline DSL (`crop(width: 1600) -> convert(to: webp)`) with sequential execution.
- Saved pipelines library (`pipeline add/list/show/run/delete`).

**Phase 4 — background daemon** ✅
- `watch` folders (`notify`): new/changed files run their attached pipeline,
  with debouncing, type filtering and loop prevention.
- `pipeline attach`/`detach` to configure folder (and clipboard) automations.
- Clipboard watcher (`--clipboard`, feature `clipboard`): copied images are
  optimised and saved to `~/Pictures/xpress`.
- Graceful Ctrl-C shutdown.

**Phase 5 — desktop GUI** ✅
- egui/eframe app with drag-and-drop, floating result cards and thumbnails.
- Background (off-thread) optimisation; compression + pipeline controls.
- Global hotkey (⌘⇧O) to optimise the clipboard image; always-on-top mode.

### Possible next steps
- Crop window / interactive crop, batch review window, uploads/sharing.
- Packaging: `.app` bundle + code signing; embed binaries via `embed-tools`.

## Development

```sh
cargo build
cargo test
cargo run -p xpress-cli -- doctor
```
