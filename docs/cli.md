# Command-line reference

```text
xpress <COMMAND> [OPTIONS] <FILES OR FOLDERS>...
```

Every command prints help with `xpress <command> --help`. The guides explain the
*why*; this page lists every command and option.

| Command | Purpose | Guide |
|---------|---------|-------|
| [`optimise`](#optimise) | Make files smaller in the same format | [Optimising](optimising.md) |
| [`convert`](#convert) | Change format (image, audio, video) | [Converting](converting.md) |
| [`downscale`](#downscale) | Shrink images and videos by a factor | [Resizing](resizing-and-cropping.md) |
| [`crop`](#crop) | Crop/resize to a size, ratio or long edge | [Resizing](resizing-and-cropping.md) |
| [`web`](#web) | Responsive image sets + `<picture>` snippet | [Web images](web-images.md) |
| [`check`](#check) | CI guard for oversized/unoptimised media | [CI](ci.md) |
| [`pipeline`](#pipeline) | Run, save and attach pipelines | [Pipelines](pipelines.md) |
| [`watch`](#watch) | Optimise new files and clipboard images automatically | [Watching](daemon.md) |
| [`history`](#history) | Search and copy from the app's clipboard history | [History](history.md#from-the-terminal-and-other-apps) |
| [`crop-pdf`, `uncrop-pdf`, `extract-pages`](#pdf-commands) | PDF tools | [PDFs](pdf.md) |
| [`restore`, `clean-backups`](#backups) | Bring back or delete `.orig` backups | [Optimising](optimising.md#backups-and-restore) |
| [`strip-exif`](#strip-exif) | Remove metadata without re-encoding | [Privacy](sharing-and-privacy.md) |
| [`config`, `doctor`, `bundle`](#information) | Settings, tools, embedded tools | [Configuration](configuration.md) |
| [`update`](#update) | Update xpress itself | [Installation](installation.md#updating) |
| `completions <shell>`, `man` | Shell completions; man page | [Installation](installation.md#shell-completions-and-man-page) |

Files, folders and globs can be mixed. Folders are read one level deep; add
`-r` to include subfolders. Sizes use decimal units (`500kb` = 500,000 bytes,
`1.5mb`, or plain bytes).

## Common options

Accepted by `optimise`, `convert`, `downscale`, `crop`, `pipeline run` and
`watch`:

| Option | Description |
|--------|-------------|
| `-r, --recursive` | Include subfolders |
| `--compression <5..100>` | The [compression dial](optimising.md#the-compression-dial): 5 = best quality, 100 = smallest. Default 30 |
| `-a, --aggressive` | The aggressive preset (64) |
| `--strip-metadata` | Remove EXIF and XMP (images) and all metadata (video); the colour profile stays |
| `--strip-location` | Remove only GPS/location; keep the rest |
| `--no-backup` | Don't keep `.<name>.orig` backups |
| `--no-preserve-dates` | Don't keep the original modification date |
| `--allow-larger` | Write the result even if it's bigger |
| `-o, --output <path>` | Output file (one input), folder (several), or [name template](optimising.md#output-options); originals untouched |
| `--force` | Re-process files [already optimised](optimising.md#skipping-files-already-optimised) with these settings |
| `--json` | Machine-readable results |
| `-q, --quiet` | Only errors and the summary |
| `-j, --jobs <n>` | Files in parallel (default: number of CPU cores) |
| `--timeout <secs>` | Stop any single external tool after this long (0 = no limit) |

## optimise

```text
xpress optimise [OPTIONS] <ITEMS>...
```

| Option | Description |
|--------|-------------|
| `--kind <image\|video\|pdf\|audio>` | Only this kind of file |
| `--quality <target>` | Images: smallest file that still meets `visually-lossless`, `high`, `medium`, `low` or a score 1–100 ([quality targets](optimising.md#quality-targets)) |
| `--max-size <size>` | Fit each file under a size ([budgets](optimising.md#size-budgets)) |
| `--for <discord\|github\|email>` | Fit a destination's limits and formats ([sharing](sharing-and-privacy.md)) |
| `--adaptive` | Images: also try JPEG/PNG and keep the smallest |
| `--pdf-dpi <36..600>` | PDFs: downsample embedded photos to this DPI ([PDFs](pdf.md)) |

`--quality`, `--max-size`, `--for` and `--adaptive` can't be combined.

```sh
xpress optimise -r ~/Screenshots
xpress optimise --quality high -r ~/Export
xpress optimise --max-size 10mb demo.mov
xpress optimise --for email --strip-location IMG_0042.HEIC
```

## convert

```text
xpress convert --to <FORMAT> [OPTIONS] <ITEMS>...
```

| Option | Description |
|--------|-------------|
| `-t, --to <format>` | Images: `png`, `jpeg`/`jpg`, `webp`, `avif`, `heic`, `gif`, `tiff`/`tif`, `bmp`, `jxl`. Audio: `aac`, `mp3`, `opus`, `wav`, `flac`, `aiff`. Video: `mp4`, `hevc`, `av1`, `webm`, `gif` |
| `--quality <target>` | JPEG/PNG/WebP: smallest output that meets the target |
| `--palette` | PNG: palette-reduced (smaller, lossy) instead of lossless |
| `--bitrate <kbit/s>` | Audio bitrate |
| `--hw` | Video: Apple silicon hardware encoder for H.264/HEVC |

The result is written next to the source, which is kept. With `--to gif`,
videos become animated GIFs and images still GIFs.

```sh
xpress convert --to jpeg *.png
xpress convert --to webp --quality high -r public/img
xpress convert --to mp3 --bitrate 128 interview.wav
xpress convert --to hevc --hw clip.mov
```

## downscale

```text
xpress downscale [-f <FACTOR>] [OPTIONS] <ITEMS>...
```

| Option | Description |
|--------|-------------|
| `-f, --factor <0.05..1.0>` | Scale factor (default 0.5) |

Images and videos only.

## crop

```text
xpress crop --size <SIZE> [OPTIONS] <ITEMS>...
```

| Option | Description |
|--------|-------------|
| `-s, --size <size>` | `1200x630` (cover-crop to exactly that), `1600x0` / `0x720` (one side, keep aspect), `16:9` (largest area of that ratio), `2000` (square) |
| `-l, --long-edge` | With a single number: make the longer side that size, no crop |
| `--smart-crop` | Images: keep the most salient region instead of the centre |

## web

```text
xpress web [OPTIONS] <IMAGES>...
```

| Option | Default | Description |
|--------|---------|-------------|
| `--widths <list>` | `640,1024,1600,2048` | Widths to generate (never upscaled) |
| `--formats <list>` | `avif,webp` | Modern formats before the JPEG/PNG fallback |
| `--quality <target>` | `high` | Perceptual target for JPEG/PNG/WebP variants |
| `--sizes <value>` | `100vw` | The `sizes` attribute |
| `--alt <text>` | empty | The `alt` text |
| `-o, --output <dir>` | `<name>-web/` | Output folder |

## check

```text
xpress check [OPTIONS] <ITEMS>...
```

Read-only; exits 1 if any file needs work.

| Option | Default | Description |
|--------|---------|-------------|
| `-r, --recursive` | | Include subfolders |
| `--max-size <size>` | | Fail for files larger than this |
| `--min-savings <pct>` | `10` | Fail when optimising would save at least this |
| `--quality <target>` | `visually-lossless` | Images: how good an optimised file must still look |
| `--compression <5..100>` | config | Video/audio/PDF measuring level |
| `--exclude <name>` | | Skip directories with this name (repeatable) |
| `--kind`, `--json`, `-q`, `-j` | | As for `optimise` |

## pipeline

```text
xpress pipeline run [OPTIONS] <PIPELINE> <ITEMS>...   # name or inline DSL; common options apply
xpress pipeline add <NAME> <DSL>                       # save
xpress pipeline list                                   # saved pipelines + automations
xpress pipeline show <NAME>
xpress pipeline delete <NAME>
xpress pipeline attach [--type <all|image|video|audio|pdf>] <SOURCE> <PIPELINE>   # folder or "clipboard"
xpress pipeline detach <SOURCE>
```

Steps and syntax: [Pipelines](pipelines.md).

## watch

```text
xpress watch [OPTIONS] [FOLDERS]...
```

| Option | Description |
|--------|-------------|
| `--clipboard` | Also optimise images copied to the clipboard |
| `-p, --pipeline <pipeline>` | Pipeline for the given folders (default `optimise`) |
| `-r, --recursive` | Watch subfolders too |

Without folders, the saved automations are used. Runs until Ctrl-C. See
[Watching](daemon.md).

## history

```text
xpress history [WORDS]... [OPTIONS]        # search (the default)
xpress history show <ID> [--text]           # print a clip
xpress history copy <ID>                    # put it on the clipboard (macOS)
xpress history pin <ID> | unpin <ID>
xpress history delete <ID>...
xpress history categories [--json]
```

| Option (search) | Description |
|--------|-------------|
| `--kind <kind>` | `text`, `link`, `code`, `color`, `image`, `screenshot`, `files`, `multi` |
| `--app <name>` | Only clips copied in this app |
| `--category <name>` | Only clips in this category |
| `--pinned` | Only pinned clips |
| `-n, --limit <n>` | How many (default 20) |
| `--json` | Machine-readable: `id`, `kind`, `title`, `text`, `ocr`, `paths`, `image`, `app`, `created`, `lastUsed`, `pinned`, `categories` |

Uses the [desktop app's history](history.md) (turn it on in the app first).
`show` prints text and file paths; for an image, its path (`--text`: the words
found in it). Changes sync to your other Macs when the app syncs.

## PDF commands

```text
xpress crop-pdf --ratio <W:H> [--suffix <text>] [-r] <ITEMS>...
xpress uncrop-pdf [-r] <ITEMS>...
xpress extract-pages [--format png|jpeg] [--dpi <n>] [-o <dir>] [-r] <ITEMS>...
```

| Option | Default | Description |
|--------|---------|-------------|
| `--ratio` | | Aspect ratio, e.g. `16:9` or `1.91:1` |
| `--suffix` | in place | Write `<name><suffix>.pdf` instead of changing the original |
| `--format` | `png` | Page image format |
| `--dpi` | `150` | Page image resolution |
| `-o, --out` | next to the PDF | Folder for page images |

`extract-pages` needs Ghostscript.

## Backups

```text
xpress restore [-r] <ITEMS>...        # put .<name>.orig backups back
xpress clean-backups [-r] <ITEMS>...  # delete them
```

Give either the files or the folders that contain the backups.

## strip-exif

```text
xpress strip-exif [-r] <ITEMS>...
```

Removes metadata from images in place without re-encoding (keeps orientation
and colour profile). Needs `exiftool`. Alternative without extra tools:
`xpress optimise --strip-metadata`.

## Information

| Command | Shows |
|---------|-------|
| `xpress config` | The config file path and the defaults in effect ([Configuration](configuration.md)) |
| `xpress doctor` | Which external tools were found |
| `xpress bundle` | Extracts tools embedded in builds made with `--features embed-tools` |

## update

```text
xpress update [--check]
```

Downloads and installs the latest release in place, after checking it against
the release's SHA-256 checksum; `--check` only reports. Uses `GITHUB_TOKEN` (or
`GH_TOKEN`) for the GitHub API when set, and the public releases page when the
API's rate limit is reached.

## Exit status

`0` on success. `1` when the command itself fails (bad options, nothing to
process), and for `check` when any file needs work. When a single file in a
batch fails, it's reported and the run continues; this doesn't change the exit
status — use `--json` to detect per-file failures in scripts.
