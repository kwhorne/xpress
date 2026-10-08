# Getting started

This page gets you from nothing to a smaller file in a few minutes, and explains
what xpress does to your files so there are no surprises.

## 1. Install

**With Homebrew:** `brew install --cask kwhorne/tap/xpress` for the app,
`brew install kwhorne/tap/xpress` for the command line. Or download them:

**macOS (recommended): the desktop app.** Download the latest
`xpress-<version>-macos-aarch64-apple-darwin.dmg` from the
[releases page](https://github.com/kwhorne/xpress/releases/latest), open it and
drag **xpress** into *Applications*. It is signed and notarised by Apple, needs
an Apple silicon or Intel Mac (pick the matching download), and includes everything it needs (also `ffmpeg` for video and
audio). The first time it opens, a short welcome tour shows the basics.

**Command line (macOS or Linux).** Download the `.tar.gz` for your platform from
the same page, unpack it and put `xpress` somewhere on your `PATH`:

```sh
tar xzf xpress-v1.0.0-aarch64-apple-darwin.tar.gz
sudo mv xpress-v1.0.0-aarch64-apple-darwin/xpress /usr/local/bin/
xpress --version
```

Images and PDFs work straight away. For **video and audio** the command line
needs `ffmpeg` — see [Installation → Extra tools](installation.md#extra-tools).

## 2. Your first optimisation

### In the app

1. Start **xpress**. It lives in the **menu bar** (the ✕ icon); its window opens
   on first launch.
2. **Drag** a photo, screenshot, video, PDF or audio file onto the window.
3. A result card appears with the new size and the saving, e.g. `−62%`.

That's it — the file was optimised **in place**, and the original was kept as a
backup (see below). Two shortcuts work from anywhere:

- **⌘⇧O** — optimise the image on the clipboard (copy large, paste small).
- **⌘⇧X** — bring the xpress window to the front.

The full tour is in [The desktop app](gui.md).

### On the command line

```sh
xpress optimise photo.jpg screenshot.png talk.mov report.pdf
```

```text
✅ photo.jpg → photo.jpg  (3.1 MB → 1.2 MB, -61%)
✅ screenshot.png → screenshot.png  (1.4 MB → 290.5 KB, -79%)
...
4 optimised, 0 failed — saved 9.8 MB (64%)
```

A whole folder, including subfolders:

```sh
xpress optimise -r ~/Desktop/Screenshots
```

## 3. What happens to your files

xpress is careful by default:

- **In place, with a backup.** The optimised file replaces the original, and
  the original is saved next to it as a hidden `.<name>.orig` file
  (e.g. `.photo.jpg.orig`). Bring originals back with
  `xpress restore <file or folder>`, or delete backups with
  `xpress clean-backups`. Turn backups off with `--no-backup` or in the app's
  Preferences.
- **Never bigger.** If the result isn't smaller, the original is left untouched
  ("already optimal"). `--allow-larger` overrides this.
- **Dates kept.** The file keeps its original modification date.
- **Photos stay correct.** Orientation, colour profile and camera metadata are
  kept (unless you ask to strip them); animated GIFs keep all their frames.
- **No double work.** xpress remembers which files it already optimised (see
  [Optimising → Skipping files](optimising.md#skipping-files-already-optimised)),
  so running it again over the same folder is instant and doesn't lose quality.
- **Format changes go next to the original.** Converting `photo.png` to JPEG
  creates `photo.jpg` and keeps `photo.png`. (One exception: optimising a `.mov`
  video produces an `.mp4` that replaces it — still with a backup.)

## 4. Next steps

- Make files **smaller still**: [Optimising](optimising.md) — compression
  levels, *quality targets* (`--quality high`) and size budgets
  (`--max-size 500kb`).
- **Change format**: [Converting formats](converting.md).
- **Send** something under a size limit: `xpress optimise --for discord clip.mov`
  — see [Sharing and privacy](sharing-and-privacy.md).
- **Automate** it: [Watching folders and the clipboard](daemon.md) and
  [Pipelines](pipelines.md).
