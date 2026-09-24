# CLI reference

```
xpress <COMMAND>
```

| Command | Purpose |
|---------|---------|
| `optimise` | Optimise images, videos, audio and PDFs |
| `downscale` | Downscale + optimise images/videos by a factor |
| `convert` | Convert images or audio to another format |
| `crop` | Crop/resize images or videos to a size or ratio |
| `pipeline` | Run, save and manage pipelines |
| `watch` | Watch folders / clipboard and optimise automatically |
| `strip-exif` | Delete EXIF metadata from images |
| `crop-pdf` | Crop PDFs to an aspect ratio (non-destructive) |
| `uncrop-pdf` | Revert a non-destructive PDF crop |
| `extract-pages` | Render PDF pages to images |
| `restore` | Restore originals from `.orig` backups |
| `clean-backups` | Delete `.orig` backups |
| `bundle` | Extract embedded binaries to the bundle dir |
| `doctor` | Report which external tools are available |
| `completions` | Print a shell completion script |
| `man` | Print a man page (roff) |

## Common options

Most commands accept these shared options:

| Option | Description |
|--------|-------------|
| `-r, --recursive` | Recurse into folders |
| `--compression <5..100>` | How hard to compress: 5 = best quality, 100 = smallest. Default 30 |
| `-a, --aggressive` | Use the aggressive preset (factor 64) |
| `--strip-metadata` | Strip non-essential metadata |
| `--no-preserve-dates` | Don't preserve original timestamps |
| `--no-backup` | Don't write a `.<name>.orig` backup |
| `--allow-larger` | Keep the result even if it is larger than the input |
| `-o, --output <PATH>` | Output file (single input) or directory (multiple inputs) |
| `-j, --jobs <N>` | Max files processed in parallel (default: number of CPUs) |
| `--timeout <SECS>` | Kill any single tool running longer than this (0 = no limit) |
| `--force` | Re-process files already marked as optimised (see below) |

While a batch runs in a terminal, a live spinner shows `[done/total]` and elapsed
time; it is suppressed under `--quiet`/`--json` or when output is piped.

Originals are backed up next to the file as `.<name>.orig` unless `--no-backup`.
Compression is a single percentage that each encoder maps to its native quality
knob (JPEG quality, PNG palette size and quality floor, libx264 CRF/preset,
audio bitrate).

**Already-optimised files are skipped.** After `optimise`, each file gets an
extended attribute (`com.xpress.optimised`, or `user.xpress.optimised` on Linux)
recording the settings and a CRC32 of its content. Re-running over the same
folder skips files whose content is unchanged and that were optimised at least
as hard — instantly, and without re-encoding (which would only add generation
loss). Editing a file, asking for more compression, `--strip-metadata` or a
lower `--pdf-dpi` makes it run again; `--force` always does. On filesystems
without extended attributes nothing is cached.

## optimise

```sh
xpress optimise [OPTIONS] <ITEMS>...
```

Auto-detects each file's type. Extra options:

- `--kind image|video|pdf|audio` — restrict to one media kind.
- `--pdf-dpi <36..600>` — downsample embedded JPEG images to at most this DPI at the size they are drawn on the page (omit to keep their resolution). Only images whose colours can be re-encoded exactly (RGB/Gray/ICC) are touched; CMYK and other colour spaces are left as they are.
- `--max-size <size>` — compress to fit a budget (`500kb`, `1.5mb`, `250000`;
  decimal units). Video and lossy audio compute the bitrate the budget allows
  from the duration and encode straight to it — two-pass H.264 for video,
  downscaled (keeping aspect, never below 240 lines) when the bits are too thin
  for the frame size — then correct if the result lands off; results typically
  land at 85–97% of the budget. Images/PDFs step up the compression until they
  fit. A file that can't get under the budget is reported with a warning.
- `--adaptive` — for images, try multiple formats and keep the smallest.
- `--quality <target>` — for images, the smallest file that still *looks* this
  good instead of a fixed compression factor. Targets are SSIMULACRA2 scores:
  `visually-lossless` (90), `high` (80), `medium` (70), `low` (50) or a number
  1–100. xpress binary-searches the compression and reports the achieved score
  (`[SSIMULACRA2 80.8]`; `"ssimulacra2"` in `--json`). Other media in the same
  run use the normal optimiser. Never grows a file.

```sh
xpress optimise photo.png clip.mov doc.pdf
xpress optimise -r --aggressive ~/Screenshots
xpress optimise --kind pdf --pdf-dpi 144 *.pdf
xpress optimise --max-size 500kb hero.jpg
xpress optimise --adaptive screenshot.png
```

### Output templates

When `--output` contains `%` tokens, it is treated as a filename template:
`%f` (stem), `%e` (extension), `%P` (parent dir), `%y%m%d`/`%H%M%S` (date/time),
`%i` (auto-increment), `%r` (random), `%%` (literal `%`).

```sh
xpress optimise -o '~/out/%f-%i.%e' *.png
xpress convert --to webp -o '%f@web.webp' *.png
```

## downscale

```sh
xpress downscale [OPTIONS] -f <FACTOR> <ITEMS>...
```

- `-f, --factor <0.05..1.0>` — scale factor (default `0.5`).

Images are scaled in pure Rust (animated GIFs are refused rather than
flattened), videos via an `ffmpeg` `scale=` filter folded into the re-encode.

```sh
xpress downscale -f 0.5 photo.png
xpress downscale -f 0.75 recording.mov
```

## convert

```sh
xpress convert [OPTIONS] -t <FORMAT> <ITEMS>...
```

- `-t, --to` — image (`webp|avif|heic|jxl|png|jpeg`), audio (`aac|mp3|opus|wav|flac|aiff`), or video (`gif|mp4|hevc|av1|webm`).
- `--bitrate <kbps>` — explicit audio bitrate.
- `--hw` — use a hardware (VideoToolbox) encoder for video on Apple Silicon.
- `--quality <target>` — for `jpeg`, `png` and `webp`: the smallest output that
  still scores the target against the original (see `optimise --quality`).
  WebP falls back to lossless when lossy can't reach the target, unless that
  would be larger than the source.

```sh
xpress convert --to webp --quality high photos/      # smallest WebP that looks "high"
xpress optimise --quality visually-lossless shot.png
```

**iPhone photos (HEIC/HEIF)** convert both ways on macOS — the built-in `sips`
is used automatically, so no extra tools are needed to read an Apple photo or to
write one.

```sh
xpress convert --to webp screenshot.png
xpress convert --to jpeg IMG_0421.HEIC     # read an iPhone photo
xpress convert --to heic screenshot.png    # write the iPhone format
xpress convert --to mp3 --bitrate 192 recording.wav
xpress convert --to gif screencast.mov
xpress convert --to hevc --hw clip.mov     # iPhone video codec
```

## crop

```sh
xpress crop [OPTIONS] -s <SIZE> <ITEMS>...
```

- `-s, --size` — `1200x630`, `1200x0`, `0x720`, aspect ratio `16:9`, or a single number.
- `-l, --long-edge` — treat a single number as the longer edge (keeps aspect, no crop).
- `--smart-crop` — for images, crop around the most salient region (detail, saturated colour, skin tones) instead of the centre. Pure Rust; videos are always cropped centred.

```sh
xpress crop --size 1200x630 banner.png
xpress crop --size 16:9 --smart-crop photo.jpg
xpress crop --size 1920 --long-edge shot.png
```

## pipeline

See [Pipeline DSL](pipelines.md).

```sh
xpress pipeline run '<dsl|name>' <ITEMS>...
xpress pipeline add <name> '<dsl>'
xpress pipeline list
xpress pipeline show <name>
xpress pipeline delete <name>
xpress pipeline attach <folder|clipboard> <pipeline> --type <all|image|video|audio|pdf>
xpress pipeline detach <folder|clipboard>
```

## watch

See [Daemon & automations](daemon.md).

```sh
xpress watch [OPTIONS] [FOLDERS]...
```

- `--clipboard` — also watch the clipboard for copied images.
- `-p, --pipeline` — pipeline (name or inline DSL) for the watched folders (default: `optimise`).

## strip-exif

```sh
xpress strip-exif [-r] <ITEMS>...
```

Removes metadata from images in place (needs `exiftool`).

## crop-pdf / uncrop-pdf / extract-pages

```sh
xpress crop-pdf --ratio 16:9 slides.pdf          # sets the page CropBox
xpress crop-pdf --ratio 1.91:1 --suffix "-cropped" doc.pdf
xpress uncrop-pdf slides.pdf                      # removes the CropBox
xpress extract-pages --format png --dpi 150 doc.pdf
```

Cropping is non-destructive (it only sets/removes the `/CropBox`), so
`uncrop-pdf` fully reverts it. `extract-pages` renders via ghostscript.

## restore / clean-backups

```sh
xpress restore [-r] <files|folders>        # move .orig backups back into place
xpress clean-backups [-r] <files|folders>  # delete .orig backups
```

## config

```sh
xpress config   # show the config file path and current defaults
```

Defaults are read from a JSON config file (`~/Library/Application Support/xpress/config.json`,
or the XDG/`APPDATA` equivalent). Command-line flags override the config; the
config overrides the built-in defaults. Recognised keys:

```json
{
  "compression": 30,
  "aggressive": false,
  "backup": true,
  "strip_metadata": false,
  "preserve_dates": true
}
```

## update

```sh
xpress update --check   # report whether a newer release exists
xpress update           # download the latest release and replace the binary
```

Checks GitHub Releases for `kwhorne/xpress`. The desktop app also shows an
“Update available” banner when a newer version is published.

## web

```sh
xpress web [OPTIONS] <IMAGES>...
```

Responsive images in one step: each image becomes several widths in modern
formats plus a fallback, and a ready-to-paste `<picture>` element (printed and
saved as `<name>.html`).

- `--widths 640,1024,1600,2048` — widths to generate; never upscales.
- `--formats avif,webp` — modern formats, in order of preference. The fallback
  is JPEG, or PNG for images with transparency.
- `--quality high` — how good JPEG/PNG/WebP variants must look (see
  `optimise --quality`); AVIF uses the compression factor.
- `--sizes "(max-width: 900px) 100vw, 900px"` — the `sizes` attribute.
- `--alt "…"` — alt text. `-o <dir>` — output directory (default `<name>-web/`).

```html
<picture>
  <source type="image/avif" srcset="hero-640.avif 640w, hero-1024.avif 1024w" sizes="100vw">
  <source type="image/webp" srcset="hero-640.webp 640w, hero-1024.webp 1024w" sizes="100vw">
  <img src="hero-1024.jpg" srcset="hero-640.jpg 640w, hero-1024.jpg 1024w" sizes="100vw"
       width="1024" height="576" alt="" loading="lazy" decoding="async">
</picture>
```

`width`/`height` are set so the page doesn't shift while images load.

## check

```sh
xpress check [OPTIONS] <ITEMS>...
```

A read-only guard for CI: exits with status 1 when a media file is over a size
limit or still unoptimised. Files are never modified — each one is optimised to
a temporary file just to measure what it could shrink to.

- `--max-size <size>` — fail for any file larger than this (`500kb`, `2mb`).
- `--min-savings <pct>` — fail for files optimising would shrink by at least
  this much (default `10`).
- `--quality <target>` — images: how good an optimised file must still look
  (default `visually-lossless`). Re-encoding a lossy JPEG always "saves"
  something by discarding more detail, so images are judged by how much smaller
  they could be *without a visible change* — an already-optimised JPEG passes.
- `--exclude <dir>` — skip directories with this name (repeatable).
- `-r`, `--kind`, `--json`, `-q`, `-j` as for `optimise`.

```sh
xpress check -r --max-size 500kb --exclude node_modules public/
```

### GitHub Action

```yaml
- uses: actions/checkout@v4
- uses: kwhorne/xpress@v0.5.0
  with:
    paths: public assets
    max-size: 500kb        # optional
    # min-savings: 10  quality: visually-lossless  exclude: node_modules .git
```

Runs on `ubuntu-latest` and `macos-latest` (it downloads the matching release
binary). A pull request that adds a 4 MB hero image or an unoptimised
screenshot then fails with the file, its size and what it could be.

## doctor / bundle

```sh
xpress doctor   # list each tool and whether it was found
xpress bundle   # extract embedded binaries (requires the embed-tools build)
```
