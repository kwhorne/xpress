# xpress documentation

xpress is an **image, video, PDF and audio optimiser** written in Rust:
a command-line tool, a background daemon, and a desktop app, all sharing one
optimisation engine.

## Contents

1. [Installation](installation.md) — building xpress and providing the tools it drives.
2. [CLI reference](cli.md) — every command and option.
3. [Pipeline DSL](pipelines.md) — chaining steps and saving/automating pipelines.
4. [Daemon & automations](daemon.md) — watching folders and the clipboard.
5. [Desktop GUI](gui.md) — drag-and-drop, hotkeys, and building a `.app`.
6. [Architecture](architecture.md) — how the crates fit together.
7. [Contributing](contributing.md) — dev workflow, tests, lints, releases.

## Quick start

```sh
cargo build --release
brew install ffmpeg pngquant jpegoptim gifsicle ghostscript vips gifski webp exiftool
./target/release/xpress doctor          # check available tools
./target/release/xpress optimise photo.png screencast.mov document.pdf
```

> xpress is an independent MIT-licensed project, inspired by the functionality of
> Clop. It contains no Clop source code — see [`../NOTICE.md`](../NOTICE.md).
