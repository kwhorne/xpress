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

While a batch runs in a terminal, a live spinner shows `[done/total]` and elapsed
time; it is suppressed under `--quiet`/`--json` or when output is piped.

Originals are backed up next to the file as `.<name>.orig` unless `--no-backup`.
Compression is a single percentage that each encoder maps to its native quality
knob (jpegoptim `--max`, pngquant `--quality`, gifsicle `-O/--lossy`, libx264
CRF/preset, audio bitrate).

## optimise

```sh
xpress optimise [OPTIONS] <ITEMS>...
```

Auto-detects each file's type. Extra options:

- `--kind image|video|pdf|audio` — restrict to one media kind.
- `--pdf-dpi <48..300>` — downsample PDF images to this DPI (omit for none).
- `--max-size <size>` — compress to fit a budget (`500kb`, `1.5mb`, `250000`).
- `--adaptive` — for images, try multiple formats and keep the smallest.

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

Images scale via `vips` (or `ffmpeg`), GIFs via `gifsicle`, videos via an
`ffmpeg` `scale=` filter folded into the re-encode.

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

```sh
xpress convert --to webp screenshot.png
xpress convert --to mp3 --bitrate 192 recording.wav
xpress convert --to gif screencast.mov
xpress convert --to hevc --hw clip.mov
```

## crop

```sh
xpress crop [OPTIONS] -s <SIZE> <ITEMS>...
```

- `-s, --size` — `1200x630`, `1200x0`, `0x720`, aspect ratio `16:9`, or a single number.
- `-l, --long-edge` — treat a single number as the longer edge (keeps aspect, no crop).
- `--smart-crop` — centre on detected features (needs `vips`).

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

## doctor / bundle

```sh
xpress doctor   # list each tool and whether it was found
xpress bundle   # extract embedded binaries (requires the embed-tools build)
```
