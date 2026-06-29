# Architecture

xpress is a Cargo workspace of three crates plus a dev-only icon tool.

```
xpress/
  crates/
    xpress-core/   # the optimisation engine (no UI, no I/O policy)
    xpress-cli/    # `xpress` binary: CLI + watch daemon
    xpress-gui/    # `xpress-gui` binary: egui desktop app
  tools/
    icon-gen/      # dev tool: render assets/icon.svg -> .iconset (excluded)
```

The engine is UI-agnostic: both the CLI and the GUI call into `xpress-core`.

## xpress-core modules

| Module | Responsibility |
|--------|----------------|
| `compression` | `CompressionQuality`: maps one 5–100 factor to each tool's native quality knob. |
| `tools` | Locate external binaries (env / sibling / bundle / PATH) and run them with retries. |
| `filetype` | Classify paths into `Image`/`Video`/`Audio`/`Pdf` by extension. |
| `image` | JPEG (jpegoptim), PNG (pngquant), GIF (gifsicle); format conversion; adaptive. |
| `video` | ffmpeg H.264 path; video→GIF; remove-audio, change-speed, cap-fps. |
| `pdf` | ghostscript (`pdfwrite`) with downsampling control. |
| `audio` | ffmpeg encoders + `AudioFormat` (aac/mp3/opus/wav/flac/aiff). |
| `scale` | Resolution downscale (vips/ffmpeg/gifsicle). |
| `crop` | Crop/resize to size, aspect ratio or long edge. |
| `budget` | Compress to a byte budget by ramping the compression factor. |
| `pipeline` | Parse + run the step DSL. |
| `template` | Expand output filename templates (`%f`, `%e`, `%i`, date/time, …). |
| `store` | Persist saved pipelines and folder automations (JSON). |
| `config` | User defaults (compression/backup/…) from `config.json`. |
| `clipboard` | Write an optimised PNG back to the clipboard (macOS). |
| `bundled` | Optional embedded binaries (`embed-tools`). |
| `result` | `OptimisationResult`, backup/dates/size helpers, `OptimiseOptions`. |

### Data flow for one file

```
path ──▶ filetype::classify ──▶ dispatch (image/video/pdf/audio)
                                   │
                                   ├─ build tool args from CompressionQuality
                                   ├─ run tool into a temp file (tools::run)
                                   ├─ size guard (skip if not smaller)
                                   ├─ backup original, copy result into place
                                   └─ OptimisationResult { old/new size, ... }
```

Pipelines copy the source into a temp working dir and apply each step to the
working file in sequence, placing the final artifact at the end (see
[pipelines](pipelines.md)).

## Binaries

- **`xpress`** (`xpress-cli`): clap-based commands; `watch` adds a `notify`
  folder watcher and an optional `arboard` clipboard watcher on a worker thread.
- **`xpress-gui`** (`xpress-gui`): an eframe app. Drops/hotkeys submit work to
  background threads that call `xpress-core` and stream results back over a
  channel; the winit event loop also powers the `global-hotkey` integration.

## Design choices

- **Shell out, don't re-implement codecs.** The mature tools do the encoding;
  xpress orchestrates them and owns the UX (one compression number, backups,
  placement, pipelines, automations).
- **One compression model.** A single 5–100 percentage drives every format, so
  presets behave consistently across images, video, PDF and audio.
- **Engine/UI split.** Everything testable lives in `xpress-core`; the binaries
  are thin.
