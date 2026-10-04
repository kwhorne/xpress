# Architecture

xpress is a Cargo workspace of three crates plus a dev-only icon tool.

```
xpress/
  crates/
    xpress-core/   # the engine: every optimisation, conversion and policy (no UI)
    xpress-cli/    # `xpress` binary: commands, `watch`, `check`, terminal output
    xpress-gui/    # `xpress-gui` binary: the egui/eframe menu-bar app
  tools/
    icon-gen/      # dev tool: render assets/icon.svg -> .iconset (excluded)
  action.yml       # the `xpress check` GitHub Action (scripts/action-check.sh)
```

Both binaries are thin: everything testable lives in `xpress-core`.

## xpress-core modules

| Module | Responsibility |
|--------|----------------|
| `compression` | `CompressionQuality`: one 5–100 factor mapped to each encoder's knob (JPEG quality, PNG palette size and quality floor, WebP/AVIF/HEIC quality, H.264 CRF/preset, audio bitrate). |
| `image` | Decoding with metadata (`load`: orientation applied, ICC/EXIF/XMP carried, animated images refused), HEIC/AVIF via `sips`/ffmpeg, JPEG (mozjpeg), PNG (quantette/exoquant + oxipng), WebP (libwebp, with our RIFF metadata muxer), AVIF (ravif + qcms → sRGB), GIF/TIFF/BMP; optimise, convert, adaptive, resize/crop, saliency crop. |
| `quality` | Perceptual targets: SSIMULACRA2 scoring and the binary search over the factor. |
| `video` | ffmpeg: banner probe (`MediaInfo`), H.264 encodes with HDR tone-mapping, codec conversion, two-pass bitrate encodes, GIF, speed/fps/audio removal, metadata policy. |
| `audio` | ffmpeg encoders + `AudioFormat`, bitrate mapping, loudness normalisation. |
| `pdf` | lopdf: colour-safe JPEG recompression, DPI downsampling from the content-stream CTM, crop/uncrop; page rendering via Ghostscript. |
| `budget` | Fit a byte budget: bitrate planning for video/audio, factor ladder otherwise. |
| `share` | `--for` presets: destination limits and format conversions. |
| `privacy` | GPS removal from EXIF and location fields from XMP. |
| `web` | Responsive image sets and the `<picture>` markup. |
| `history` | Clipboard/screenshot history: SQLite + FTS5 search, kind detection (text/link/code/colour), image storage and previews, retention. |
| `ocr` | Text in images via Apple Vision (macOS). |
| `scale`, `crop` | Downscale and size/ratio/long-edge/rect crops (images in-process, video via ffmpeg filters). |
| `effects` | Watermark overlay (ffmpeg). |
| `pipeline` | Parse and run the step DSL. |
| `cache` | "Already optimised" markers (xattr: settings + CRC32). |
| `result` | `OptimiseOptions`, `OptimisationResult`, backups, and `finish()`/`place_file()` — the shared, atomic final step. |
| `tools` | Locate and run external binaries (with timeouts). |
| `filetype`, `template`, `config`, `store`, `clipboard`, `update`, `bundled` | Extension classification, output-name templates, `config.json`, `pipelines.json`, writing PNGs to the clipboard, release checks, optional embedded binaries. |

### One file, start to finish

```
path ─▶ optimise_file ─▶ cache::is_optimised? ──yes──▶ result (cached)
                 │
                 ▼  dispatch on filetype (image / video / pdf / audio)
          encode into a temp file (in-process codec or ffmpeg)
                 │
                 ▼  result::finish(placement)
          size guard ─▶ backup original ─▶ place_file (temp + fsync + rename,
          keeps permissions/xattrs) ─▶ copy dates ─▶ remove replaced source
                 │
                 ▼
          cache::mark ─▶ OptimisationResult
```

Every operation (optimise, convert, crop, budget, pipelines, quality targets)
ends in `result::finish`, so backups, the never-bigger rule and atomic writes
behave the same everywhere.

Pipelines copy the source to a temp working directory, apply each step to the
working file, and place the final artifact with the same `finish` rules (see
[Pipelines](pipelines.md)).

## Binaries

- **`xpress`** (`xpress-cli`): clap commands; `progress` runs jobs in parallel
  (rayon) with a live counter; `render` prints results and `--json`; `watch`
  combines a `notify` folder watcher (with settle detection and loop
  prevention) and an `arboard` clipboard watcher; `check` is the read-only CI
  guard.
- **`xpress-gui`**: an eframe app. `logic()` handles the tray, global hotkeys
  and finished jobs even while the window is hidden; `ui()` draws. Work runs on
  background threads (`work.rs`) that call `xpress-core` and send results back
  over a channel. `settings.rs` persists `gui.json`. The clipboard history is
  `pasteboard.rs` (NSPasteboard: read/write, private-content markers, front
  app), `capture.rs` (a background thread polling the pasteboard's change
  count and the screenshot folder, recording into `history` and running OCR)
  and `history_ui.rs` (the view). UI tests drive it headlessly with
  `egui_kittest`; pasteboard tests use a private pasteboard, never the user's
  clipboard.

## Design choices

- **Correctness over raw savings.** Orientation, colour profiles, animation and
  metadata survive by default; nothing is overwritten with a bigger file;
  writes are atomic; backups come first.
- **In-process where it matters.** Images and PDFs use linked, permissively
  licensed libraries (no GPL code in the binaries); ffmpeg is driven as a
  separate process for video and audio.
- **One compression model**, plus perceptual targets and size budgets for when a
  number isn't what you care about.
- **Engine/UI split.** The CLI and the app share all behaviour through
  `xpress-core`.
