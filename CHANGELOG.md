# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.3] - 2026-10-06

### Fixed
- Copies made by xpress itself (`xpress history copy`, `xpress watch
  --clipboard`, *Optimise clipboard*) are no longer recorded again by the
  app's history — which also changed a clip's source app to Terminal.
- The first sync with many images no longer makes the app hang: images are
  copied to iCloud Drive without holding the history.
- Apple Intelligence shows up without restarting xpress once it's turned on
  (or has finished getting ready).

## [0.8.2] - 2026-10-05

### Added
- **Linux: the clipboard history records images and files** (not only text),
  and new screenshots from `~/Pictures/Screenshots`; copying back restores
  images, files and multi-clips (text with HTML). Wayland compositors with the
  data-control protocol are supported as well as X11.

## [0.8.1] - 2026-10-04

### Added
- **Ignore apps** for the clipboard history: nothing copied in them is
  recorded. Add them in Preferences (from the apps you've copied from, or any
  app in /Applications) or right-click a clip → *Don't record from …*; delete
  the clips an app already left with one button.

## [0.8.0] - 2026-10-04

### Added
- **`xpress history`**: search the app's clipboard history from the terminal
  (by words, kind, app, category, pinned; `--json` for scripts, Raycast, Alfred
  and Shortcuts), print a clip (`show`), put it back on the clipboard (`copy`),
  pin, unpin and delete. `XPRESS_HISTORY_DIR` points both at another folder.
- **Shortcuts can be changed** in Preferences → Shortcuts (or turned off):
  click, press the new keys; conflicts and shortcuts another app holds are
  reported. The menu-bar menu and the drop zone show the current ones.

## [0.7.1] - 2026-10-04

### Fixed
- *Paste directly* presses the key that gives ⌘V in the current keyboard
  layout; with plain Dvorak it pressed the wrong key.
- Result cards fit very wide or tall images into the same preview size instead
  of pushing the text aside.
- New app screenshot (with History in the sidebar).

## [0.7.0] - 2026-10-04

### Added
- **Apple Intelligence** in the clipboard history: summarise, proofread, make
  shorter / professional / friendly, or translate a clip to English with the
  on-device model (macOS 26+, Apple silicon), then copy the result or save it
  as a new clip. A small Swift helper, `xpress-ai`, is bundled with the app.
- **Sync with iCloud**: the clipboard history — clips, images, multi-clips,
  pins and categories — follows you between Macs through an `xpress` folder in
  iCloud Drive. Each Mac keeps its own retention; deletions sync.

## [0.6.0] - 2026-10-04

### Added
- **Clipboard and screenshot history** in the desktop app: everything you copy
  (text, links, code, colours, images, files) and every new screenshot, in a
  searchable list with previews — filter by kind or source app, pin clips, and
  copy one back with ⌃⌘V → type → ⏎ (xpress steps aside so you can ⌘V).
  Words **inside images** are searchable through on-device text recognition
  (Apple Vision). Off until turned on; content password managers mark as
  private, and copies made in password managers, are never recorded. Kept for
  a chosen time (default a month, pinned clips forever, at most 2 GB) in a
  local SQLite database. See [docs/history.md](docs/history.md).
- **Multi-clips**: pick several clips (⌘-click, ⇧-click) and *Copy together*,
  or *Collect* everything you copy into one — text, links, images and files
  from different apps, pasted at once (plain text in text fields; text and
  images in rich editors; all files in Finder).
- **Categories** for the clipboard history, with colours and optional rules
  (source app, kind, words — also words found in images) that sort new and
  existing clips automatically; filter by category.
- **Paste directly**: choose a clip and xpress pastes it into the app you were
  using (needs the Accessibility permission).
- ⌘1–⌘9 copy the first nine clips; *Collect clips* in the menu-bar menu.

### Fixed
- Preferences scroll when they're taller than the window.

## [0.5.4] - 2026-10-04

### Added
- **The desktop app remembers its settings** between launches — compression,
  *Aggressive*, *Convert to*, the pipeline and all Preferences — in `gui.json`
  next to `config.json`, saved automatically. The first launch starts from
  `config.json`.

## [0.5.3] - 2026-10-04

### Fixed
- **Updates work when GitHub's API rate limit is reached** (60 anonymous
  requests an hour per network address): the update check falls back to the
  public releases page, and uses `GITHUB_TOKEN`/`GH_TOKEN` when set. Previously
  `xpress update` failed with `status code 403`.
- The `cargo audit` CI job failed on every push to `main` because it couldn't
  report its result.

### Changed
- `xpress update` and the app's *Update & Restart* verify the download against
  the release's SHA-256 checksum before replacing anything. The CLI no longer
  depends on `self_update`.

## [0.5.2] - 2026-10-04

### Added
- **Complete user guide** in [`docs/`](docs/README.md): getting started,
  optimising, converting, resizing and cropping, video and audio, PDFs, web
  images, sharing and privacy, CI, formats, configuration and troubleshooting,
  plus a full command-line reference. New app screenshot.

### Fixed
- `-o` with an output template now creates missing folders instead of failing.
- `convert --to mp4 --hw` uses the hardware H.264 encoder on Apple silicon (it
  was ignored for H.264).
- AVIF files can now be optimised (re-encoded with the compression dial); they
  were reported as unsupported.
- `targetSize(kb: 300)` in pipelines means 300 KB (it was read as 300 bytes).
- `xpress watch --clipboard` uses the pipeline attached to `clipboard` when no
  `--pipeline` is given.
- `xpress doctor` lists gifsicle; `--adaptive` and `--max-size` are rejected
  together; clearer help for `downscale --factor` and `convert`.

## [0.5.1] - 2026-10-04

### Added
- **Image conversion in the desktop app**, Clop-style: a *Convert to* picker on
  the Optimise screen (applies to everything dropped, opened or pasted), a
  format chip on each image result (`PNG ▾` → convert to another format), and a
  right-click menu on results (*Convert to*, *Crop…*, *Show in Finder*,
  *Copy*). Converted files are saved next to the originals.
- **More formats**: convert to **GIF, TIFF and BMP**, and read **AVIF** (also
  for crop, resize and quality targets) — every readable image now converts to
  every writable format. `convert --to gif` handles images as well as videos.

### Changed
- `convert --to png` (and PNG in the desktop app) is now **lossless**; pass
  `--palette` for the smaller, palette-reduced PNG it used to produce.

### Fixed
- **`xpress update` on macOS** has not worked since the app zip joined the
  release assets (0.4.4): the updater takes the first asset, alphabetically,
  containing the target triple, which was the `-app.zip`, not the CLI tarball.
  The app zip and DMG are now named `xpress-<tag>-macos-<target>…` so the
  tarball comes first (which also fixes the update path for already-installed
  CLIs), and the CLI asks for the `.tar.gz` explicitly.

## [0.5.0] - 2026-09-24

### Added
- **`--for discord|github|email`** share presets: converts formats the
  destination can't show and compresses to its size limit (hard-coded from the
  services' published limits as of September 2026 — Discord 20 MB, GitHub
  10 MB images/video, email ~14 MB to survive base64 in Gmail and Outlook).
  A 20 s 1080p clip came out at 18.8 / 9.1 / 13.5 MB.
- **`--strip-location`** (and *Remove location* in the desktop app): removes
  only where a photo or video was taken — the EXIF GPS block is wiped (not just
  unlinked, so no coordinates linger in the bytes), XMP location fields are
  dropped, and video `location` metadata is blanked — while camera, date,
  orientation and colour profile stay. `--strip-metadata` now also removes
  video metadata (it did nothing for video before).
- **`xpress web`** — responsive images from one source: several widths
  (never upscaled) in AVIF and WebP plus a JPEG/PNG fallback at a perceptual
  quality target, and a ready-to-paste `<picture>` element with `srcset`,
  `sizes`, `width`/`height` and lazy loading.
- **`xpress check`** — a read-only CI guard that exits 1 when media files are
  over `--max-size` or could shrink by `--min-savings` (default 10%). Images are
  judged against a perceptual target (visually lossless by default), so
  already-optimised JPEGs pass instead of being flagged forever. Plus a
  **GitHub Action** (`uses: kwhorne/xpress@…`) that runs it in one step.
- **Desktop app:** a *Quality target* in Preferences (visually lossless /
  high / medium / low — the smallest image that still looks that good, with the
  SSIMULACRA2 score on the result card) and a *Skip already-optimised files*
  toggle. The compression slider is disabled while a quality target is set.

### Fixed
- Sizes are shown in decimal units (1 KB = 1000 bytes), matching how they are
  parsed — `--max-size 300kb` used to be reported as "293.0 KB".
- Desktop app: `⇧` in the hotkey hints and `→` on result cards rendered as
  boxes; the macOS Apple Symbols font is now a fallback.
- Desktop app: sidebar items are now exposed to screen readers (VoiceOver).

### Changed
- **JPEG encoding uses mozjpeg** (progressive, trellis quantisation, optimised
  Huffman; baseline inside PDFs). At equal perceptual quality (SSIMULACRA2 85 /
  75) photos and screenshots came out 35–61% smaller than with 0.4.9's encoder;
  one synthetic image of saturated coloured edges gained only ~5% (and at 85 its
  original is kept). High settings (quality ≥ 90) keep full-resolution chroma,
  and the gentlest compression factors now reach JPEG quality 98.
- The macOS release tarball's `xpress` and `xpress-gui` binaries (also what
  `xpress update` installs) are now Developer ID signed with the hardened
  runtime and notarised, like the `.app`/`.dmg`; they used to be ad-hoc signed.

## [0.4.9] - 2026-09-24

### Added
- **`--max-size` for video and audio now targets the size directly.** The
  bitrate is computed from the budget and duration and encoded two-pass
  (video), with the frame downscaled when the bits are too thin and up to three
  corrections — instead of up to six CRF re-encodes that couldn't aim at a
  size. Real 20 s 1080p clips landed at 87–93% of 10 MB / 4 MB / 1.5 MB budgets;
  MP3/AAC at 87–97%. Files still over budget get a warning.
- **Perceptual quality targets.** `optimise --quality high` (or
  `visually-lossless`, `medium`, `low`, or a score 1–100) and
  `convert --to webp|jpeg|png --quality …` find the smallest file that still
  meets a SSIMULACRA2 score against the original, instead of guessing a
  compression factor, and report the score achieved.
- **Already-optimised files are skipped.** `optimise` marks each result with an
  extended attribute (settings + CRC32 of the content); re-running over a
  folder skips unchanged files that were optimised at least as hard — instantly
  and without piling up generation loss. `--force` re-processes; `--json`
  reports `"cached"`.
- `--smart-crop` (and `smart: true` in pipelines) now works for images: a
  pure-Rust saliency crop (edges, saturation, skin tones) picks the most
  interesting region instead of the centre. It previously did nothing.
- `--pdf-dpi` now works: embedded JPEG images are downsampled to at most that
  DPI at the size they are actually drawn (read from the page content streams,
  falling back to the page size).

### Fixed
- PDF optimisation no longer re-encodes CMYK (or Lab/Indexed/Separation) JPEGs
  — decoding turned them into RGB while the PDF still declared CMYK, corrupting
  the colours. Gray images stay 1-component JPEGs.
- Shrink-only pipelines (e.g. the watch daemon's default `optimise`) no longer
  replace a file with a bigger result.
- **Animated GIFs were flattened to their first frame** (and reported as a big
  saving). Animated GIF/WebP/APNG are now never decoded to a single frame: GIFs
  go through `gifsicle` when installed, otherwise they are left untouched;
  resize/crop/convert of an animated image fails instead of dropping frames.
- **Photos lost their orientation, colour profile and EXIF** when optimised,
  converted, resized or cropped — portrait iPhone shots came out sideways and
  Display-P3 images washed out. The EXIF orientation is now applied to the
  pixels (tag reset), and the ICC profile and EXIF are carried through.
  `--strip-metadata` drops the EXIF but keeps the colour profile.
- **AVIF conversion** failed with "format not supported"; it now works (pure
  Rust, `ravif`), with quality taken from the compression value.
- **WebP conversion** was lossless-only (often larger than the PNG); it is now
  lossy via libwebp, keeps alpha and the ICC profile.
- XMP metadata (ratings, captions, edit history) is kept through JPEG, PNG and
  WebP outputs, and WebP now also carries EXIF; `--strip-metadata` drops XMP.
- AVIF output (which can't embed an ICC profile) is converted to sRGB from the
  source's colour profile, so Display-P3 and other wide-gamut images no longer
  shift colour.
- Transparent images converted to JPEG are flattened onto white instead of
  black.
- `downscale`/`crop` of an image **backed up the already-modified file**, so
  `restore` could not bring the original back. The backup is now taken first.
- Optimising a `.mov` no longer replaces it with a *larger* `.mp4` (and deletes
  the original); video conversions back the source up before removing it.
- H.264 output is always 8-bit 4:2:0, so iPhone HDR (10-bit) clips no longer
  become High-10 files that QuickTime/Safari can't play — and HDR sources (HLG
  or PQ) are tone-mapped to SDR BT.709 (zscale + mobius) instead of coming out
  grey and washed out. Falls back to a plain encode if ffmpeg lacks zscale;
  HEVC/AV1/VP9 conversions keep HDR untouched.
- AAC encoding falls back to ffmpeg's native `aac` encoder when `aac_at`
  (macOS AudioToolbox) is unavailable — e.g. on Linux.
- `watch`: files are only processed once they stop changing (no more
  half-copied files), and a run's own output (e.g. `clip.mp4`, `photo.webp`) no
  longer triggers a second run.
- `watch --clipboard`: the optimised image put back on the clipboard is no
  longer picked up and re-optimised in a loop; clipboard images are written to
  PNG in pure Rust (no ffmpeg needed).

### Changed
- **`convert` keeps the source for video and audio**, like it already did for
  images: the result is written alongside. Previously a video/audio conversion
  deleted the original (irrecoverably with `--no-backup`). Optimising a `.mov`
  to `.mp4` still replaces it. An audio conversion to a bigger format (e.g.
  MP3 → FLAC) is no longer refused by the size guard.
- Dependencies: `lopdf` 0.45 (fixes RUSTSEC-2026-0187, a stack overflow on
  crafted PDFs), `self_update` 1.x without the S3 backend, `egui`/`eframe`
  0.36, and a `cargo update` — `cargo audit` reports no vulnerabilities.
- An explicit MP3 bitrate (`lowerBitrate(kbps: …)`, size budgets) now encodes
  CBR at that bitrate instead of the nearest VBR quality level, which could land
  far from it.
- WebP encodes use libwebp's sharp RGB→YUV conversion, keeping coloured edges
  crisp; compression factors below 30 now reach WebP/HEIC/AVIF quality 95
  (previously capped at 72). The normal preset is unchanged.
- **Licence:** PNG quantisation no longer uses GPL-3.0 `libimagequant`
  (statically linked, which made the MIT binaries effectively GPL). Opaque PNGs
  now use `quantette` (k-means in Oklab, dithered), PNGs with transparency use
  `exoquant`; on our samples results are smaller for screenshots/icons and ~6 %
  larger for photos, with no visible difference. A PSNR floor
  derived from the compression value keeps an image lossless when a palette
  would cost too much quality. MSRV is now Rust 1.90.
- All outputs are written atomically (temp file + rename next to the
  destination, keeping permissions and, on macOS, Finder tags/xattrs), so a
  crash mid-write can't leave a truncated file.

## [0.4.8] - 2026-07-01

### Added
- **iPhone photo (HEIC/HEIF) conversion** on macOS — both directions. Read an
  Apple photo (`convert --to jpeg IMG.HEIC`) or write the iPhone format
  (`convert --to heic photo.png`), and optimise `.heic` files in place. Uses the
  built-in `sips`, so no extra tools are required.

### Changed
- `convert` now reports produced files clearly even when the new format is
  larger than the source (instead of the misleading “already optimal”).

## [0.4.7] - 2026-07-01

### Added
- Global hotkey **⌘⇧X** brings the xpress window to the front from anywhere
  (handy for the menu-bar app). ⌘⇧O still optimises the clipboard. Both are
  shown in the menu-bar menu.

## [0.4.6] - 2026-07-01

### Changed
- The macOS app is now a **pure menu-bar (agent) app** — no Dock icon
  (`LSUIElement`). It lives in the menu bar and brings its window to the front
  when opened.

## [0.4.5] - 2026-07-01

### Added
- The desktop app now lives in the **macOS menu bar**: a status-bar icon with a
  menu (Open, Optimise clipboard, Check for updates, Quit). Clicking it shows the
  window. Closing the window hides it to the menu bar instead of quitting (use
  the menu's “Quit” to exit), so xpress stays a click away.

## [0.4.4] - 2026-07-01

### Changed
- The desktop app now **updates itself**: the “Update available” banner (and the
  About dialog) show an **Update & Restart** button that downloads the new
  release, replaces the installed `.app`, and relaunches automatically — no more
  bouncing to the GitHub releases page. (macOS; falls back to a download link
  when not running from an installed `.app`.)

## [0.4.3] - 2026-07-01

### Changed
- PDF optimisation is now **pure Rust**: embedded JPEG images are recompressed
  via the image engine and streams are losslessly re-compressed with `lopdf` —
  no `ghostscript` needed. (`gs` is still used only by `extract-pages`.)
  Images + video + audio + PDF now all work with no external tools to install
  (video/audio via the bundled ffmpeg).

## [0.4.2] - 2026-07-01

### Changed
- Images are now optimised, resized, cropped and converted in **pure Rust**
  (`imagequant` + `oxipng` + the `image` crate) — no external tools
  (pngquant/jpegoptim/gifsicle/vips/cwebp) are required for images anymore.

### Added
- The macOS `.app`/`.dmg` now **bundle a self-contained `ffmpeg`** (video +
  audio), signed and notarised alongside the app. Combined with the pure-Rust
  image engine, images + video + audio work out of the box with nothing to
  install. (`scripts/fetch-static-tools.sh`, `make-app.sh --bin-dir`.)
  PDF still uses an external `ghostscript` if present.

## [0.4.1] - 2026-07-01

### Changed
- Redesigned the desktop GUI: a left sidebar with sections and colourful icon
  tiles, roomy tabbed views (Optimise / Preferences / About), rounded cards,
  macOS-style toggle switches, and a Tokyo Night colour palette.

### Added
- macOS code signing + notarisation: `make-app.sh`/`make-dmg.sh` sign with a
  Developer ID (hardened runtime) when available, a `notarize.sh` helper submits
  and staples, and the release workflow signs + notarises when the signing
  secrets are configured. See `docs/signing.md`.
- Auto-update: `xpress update` checks GitHub Releases and replaces the binary in
  place (`--check` to only report). The desktop app checks on launch and then
  periodically (every 6h), shows an “Update available” banner when a newer
  release is published, and has a “Check for updates” button in the About dialog.

## [0.4.0] - 2026-06-29

### Added
- `--timeout <secs>` kills any external tool that runs too long (prevents hangs).
- `--jobs <n>` caps how many files are processed in parallel.
- `completions <shell>` and `man` commands to generate shell completions and a
  man page.

### Changed
- CI now runs a real-tool smoke test (ffmpeg/pngquant/…) and `cargo audit`.

### Fixed
- `crop-pdf` had a conflicting `-r` short flag (`--ratio` vs `--recursive`);
  `--ratio` is now long-only.

### Added (tests)
- Failure-path tests (missing file, unsupported type, unreadable dimensions) and
  an isolated external-tool timeout test.

## [0.3.0] - 2026-06-29

> Supersedes the briefly-tagged 0.2.1: these are new features, so they belong in
> a minor release under SemVer.

### Added
- GUI: an interactive **crop** tool (drag a region, apply), plus **Reveal** and
  **Copy** actions on each result card.
- `crop::crop_rect` in the engine for arbitrary normalised-rectangle crops.
- Integrations guide (Shortcuts via Run Shell Script, Folder Actions, Photos
  export flow, uploads via `runScript`).
- PDF: non-destructive `crop-pdf` (sets the page CropBox) and `uncrop-pdf`
  (removes it), plus `extract-pages` to render pages to PNG/JPEG.
- New pipeline steps: `normalize(lufs:)` (audio loudness), `watermark(image:,
  position:, opacity:, scale:)`, `copyToClipboard`, and `runScript(code:|path:)`.
- Video codec conversion: `convert --to mp4|hevc|av1|webm` (and the same in the
  pipeline DSL), with a `--hw` flag for VideoToolbox on Apple Silicon.
- Adaptive image optimisation is now transparency-aware: PNGs with an alpha
  channel never get a JPEG candidate (no silent flattening).

## [0.2.0] - 2026-06-29

### Added
- Live progress: batch commands show a spinner with a `[done/total]` counter and
  elapsed time on a terminal (suppressed under `--quiet`/`--json` and when piped).
- Clipboard “paste small”: on macOS the optimised image is written back to the
  clipboard as PNG (in both `watch --clipboard` and the GUI's ⌘⇧O).
- `convert --to gif` (and pipeline `convert(to: gif)`) to turn videos into GIFs
  (gifski when available, otherwise ffmpeg).
- `optimise --max-size <budget>` and a `targetSize(bytes:)` pipeline step to
  compress to a byte budget.
- `optimise --adaptive` and an `adaptive` pipeline step that try multiple image
  formats and keep the smallest.
- Output filename templates for `--output` (`%f`, `%e`, `%P`, date/time, `%i`,
  `%r`, `%%`).
- `--json` and `--quiet` output modes.
- `restore` and `clean-backups` commands to manage `.orig` backups.
- A user config file (`config.json`) for default compression and behaviours, plus
  a `config` command to show it.
- Integration test suite using stub tools.
- MSRV declared (Rust 1.87) and a `rust-toolchain.toml`; GitHub issue/PR templates.

### Fixed
- `convert` to PNG/JPEG no longer optimises a file in place onto itself.
- Hardened path handling (no more `unwrap()` on `file_name`/`file_stem`).

## [0.1.0] - 2026-06-29

### Added
- **Core engine** (`xpress-core`): a percentage-based compression model and tool
  runner that drives `ffmpeg`, `pngquant`, `jpegoptim`, `gifsicle`, `ghostscript`,
  `vips`, `gifski`, `cwebp`, `heif-enc`, `cjxl` and `exiftool`.
  - Image (jpeg/png/gif), video (H.264), PDF (ghostscript) and audio optimisation.
  - Resolution scaling (`downscale`) for images and videos.
  - Format conversion: images (webp/avif/heic/jxl/png/jpeg) and audio
    (aac/mp3/opus/wav/flac/aiff).
  - Crop to a size, aspect ratio or long edge (vips smart crop / ffmpeg).
  - A pipeline DSL (`crop(width: 1600) -> convert(to: webp)`) with sequential
    execution, plus a saved-pipeline library and folder automations.
  - Parallel batch optimisation, backups, size guard, metadata and timestamp
    preservation.
- **CLI** (`xpress`): `optimise`, `downscale`, `convert`, `crop`, `pipeline`
  (run/add/list/show/delete/attach/detach), `watch`, `strip-exif`, `bundle`,
  `doctor`.
- **Daemon**: `watch` monitors folders (and optionally the clipboard) and runs
  the attached pipeline automatically, with debouncing and loop prevention.
- **Desktop GUI** (`xpress-gui`): egui/eframe app with drag-and-drop, floating
  result cards and thumbnails, a global hotkey (⌘⇧O) to optimise the clipboard
  image, always-on-top mode, and off-thread processing.
- **Binary bundling**: per-user bundle dir resolution, `scripts/fetch-tools.sh`,
  and an `embed-tools` feature that bakes binaries into the executable.
- **CI/Release**: GitHub Actions for fmt + clippy + tests, and tagged releases
  building macOS (arm64/x86_64) and Linux binaries, a macOS `.app` zip, and a
  macOS `.dmg` (with an Applications shortcut for drag-installing).

### Notes
- xpress is an independent project under the MIT License, inspired by the
  functionality of Clop. It contains no Clop source code. See `NOTICE.md`.

[Unreleased]: https://github.com/kwhorne/xpress/compare/v0.8.3...HEAD
[0.8.3]: https://github.com/kwhorne/xpress/compare/v0.8.2...v0.8.3
[0.8.2]: https://github.com/kwhorne/xpress/compare/v0.8.1...v0.8.2
[0.8.1]: https://github.com/kwhorne/xpress/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/kwhorne/xpress/compare/v0.7.1...v0.8.0
[0.7.1]: https://github.com/kwhorne/xpress/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/kwhorne/xpress/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/kwhorne/xpress/compare/v0.5.4...v0.6.0
[0.5.4]: https://github.com/kwhorne/xpress/compare/v0.5.3...v0.5.4
[0.5.3]: https://github.com/kwhorne/xpress/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/kwhorne/xpress/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/kwhorne/xpress/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/kwhorne/xpress/compare/v0.4.9...v0.5.0
[0.4.9]: https://github.com/kwhorne/xpress/compare/v0.4.8...v0.4.9
[0.4.8]: https://github.com/kwhorne/xpress/compare/v0.4.7...v0.4.8
[0.4.7]: https://github.com/kwhorne/xpress/compare/v0.4.6...v0.4.7
[0.4.6]: https://github.com/kwhorne/xpress/compare/v0.4.5...v0.4.6
[0.4.5]: https://github.com/kwhorne/xpress/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/kwhorne/xpress/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/kwhorne/xpress/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/kwhorne/xpress/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/kwhorne/xpress/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/kwhorne/xpress/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/kwhorne/xpress/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/kwhorne/xpress/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kwhorne/xpress/releases/tag/v0.1.0
