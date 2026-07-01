# xpress documentation

xpress is an **image, video, PDF and audio optimiser** written in Rust:
a command-line tool, a background daemon, and a desktop app, all sharing one
optimisation engine.

## Highlights

- Optimise images, video, PDF and audio with one compression value (5–100).
- Downscale, crop (size/ratio/long-edge), and convert formats — images, audio,
  and video codecs (`mp4`/`hevc`/`av1`/`webm`/`gif`).
- Non-destructive PDF crop/uncrop and page extraction.
- Compress to a byte budget (`--max-size`) and pick the smallest format (`--adaptive`).
- Pipelines: chain steps (`crop(width: 1600) -> convert(to: webp)`), save them,
  and attach them to folders or the clipboard.
- Background `watch` daemon for folders and the clipboard (“copy large, paste small”).
- Desktop GUI with drag-and-drop, live results, and a global hotkey.
- Output filename templates, `.orig` backups with `restore`, `--json`/`--quiet`,
  and live progress.

## Contents

1. [Installation](installation.md) — building xpress and providing the tools it drives.
2. [CLI reference](cli.md) — every command and option.
3. [Pipeline DSL](pipelines.md) — chaining steps and saving/automating pipelines.
4. [Daemon & automations](daemon.md) — watching folders and the clipboard.
5. [Desktop GUI](gui.md) — drag-and-drop, hotkeys, and building a `.app`.
6. [Integrations](integrations.md) — Shortcuts, Folder Actions, Photos, uploads.
7. [Architecture](architecture.md) — how the crates fit together.
8. [Contributing](contributing.md) — dev workflow, tests, lints, releases.

## Quick start

```sh
cargo build --release
brew install ffmpeg pngquant jpegoptim gifsicle ghostscript vips gifski webp exiftool
./target/release/xpress doctor          # check available tools
./target/release/xpress optimise photo.png screencast.mov document.pdf
```

See [`../CHANGELOG.md`](../CHANGELOG.md) for the release history (current: 0.4.0).

---

Developed by **Knut W. Horne** · [kwhorne.com](https://kwhorne.com)

> xpress is an independent MIT-licensed project, inspired by the functionality of
> Clop. It contains no Clop source code — see [`../NOTICE.md`](../NOTICE.md).
