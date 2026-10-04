# xpress documentation

xpress makes images, videos, PDFs and audio smaller — without making them look
or sound worse. Use it as a **menu-bar app** on macOS (drag files in), from the
**command line**, or as a **background watcher** that optimises new files for you.

New here? Start with **[Getting started](getting-started.md)**.

## User guide

| Guide | What it covers |
|-------|----------------|
| [Getting started](getting-started.md) | Install, your first optimisation, what happens to your files |
| [Installation](installation.md) | Every way to install, update and uninstall; extra tools |
| [The desktop app](gui.md) | Every screen, button and setting of the macOS app |
| [Optimising](optimising.md) | Compression levels, quality targets, size budgets, backups, skipping work already done |
| [Converting formats](converting.md) | PNG ↔ JPEG ↔ WebP ↔ AVIF ↔ HEIC ↔ GIF ↔ TIFF ↔ BMP, and which to choose |
| [Resizing and cropping](resizing-and-cropping.md) | Downscale, crop to a size or ratio, smart crop |
| [Video and audio](video-and-audio.md) | Codecs, HDR, GIFs, fitting a size, audio formats |
| [PDFs](pdf.md) | Shrinking PDFs, downsampling images, cropping, page images |
| [Images for the web](web-images.md) | Responsive AVIF/WebP/JPEG sets with a `<picture>` snippet |
| [Sharing and privacy](sharing-and-privacy.md) | Fitting Discord/GitHub/email limits; removing location and metadata |
| [Pipelines](pipelines.md) | Chaining steps (`crop → convert → …`), saving them |
| [Watching folders and the clipboard](daemon.md) | Automatic optimisation in the background |
| [CI and repositories](ci.md) | `xpress check` and the GitHub Action |
| [Integrations](integrations.md) | Shortcuts, Finder Quick Actions, Photos, uploads |
| [Troubleshooting & FAQ](troubleshooting.md) | Common questions and problems |

## Reference

| Reference | |
|-----------|--|
| [Command-line reference](cli.md) | Every command and option |
| [Supported formats](formats.md) | What can be read, written and optimised |
| [Configuration](configuration.md) | Config files, defaults, environment variables, where things are stored |

## For developers

[Architecture](architecture.md) · [Contributing](contributing.md) ·
[Code signing & notarisation](signing.md) · [Changelog](../CHANGELOG.md)

---

Developed by **Knut W. Horne** · [kwhorne.com](https://kwhorne.com).
xpress is MIT-licensed and an independent project inspired by the ideas of
[Clop](https://lowtechguys.com/clop); it contains no Clop source code — see
[`NOTICE.md`](../NOTICE.md).
