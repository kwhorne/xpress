# Installation

## Build from source

Requires a recent stable Rust toolchain.

```sh
git clone https://github.com/kwhorne/xpress.git
cd xpress
cargo build --release
# binaries:
#   target/release/xpress       (CLI + daemon)
#   target/release/xpress-gui   (desktop app)
```

Install the CLI onto your `PATH`:

```sh
cargo install --path crates/xpress-cli
```

## Providing the optimisation tools

xpress does not re-implement codecs; it drives external binaries. Each command
needs the tool(s) for the file types you use:

| Tool          | Used for                         |
|---------------|----------------------------------|
| `ffmpeg`      | video, audio, image scaling fallback |
| `pngquant`    | PNG                              |
| `jpegoptim`   | JPEG                            |
| `gifsicle`    | GIF                             |
| `ghostscript` (`gs`) | PDF                       |
| `vips` / `vipsthumbnail` | image resize / smart crop |
| `gifski`      | video → GIF                     |
| `cwebp`       | WebP conversion                 |
| `heif-enc`    | HEIC / AVIF conversion          |
| `cjxl`        | JPEG XL conversion              |
| `exiftool`    | metadata copy / strip           |

### Resolution order

For every tool, xpress looks in:

1. `$XPRESS_BIN_DIR/<tool>`
2. a `bin/` directory next to the `xpress` executable
3. the per-user bundle dir
   (`~/Library/Application Support/xpress/bin`, or the XDG/`LOCALAPPDATA` equivalent)
4. the system `PATH`

Check what is found with:

```sh
xpress doctor
```

### Three ways to provide tools

**A — package manager (simplest):**

```sh
brew install ffmpeg pngquant jpegoptim gifsicle ghostscript vips gifski webp exiftool
# Linux: apt-get install ffmpeg pngquant jpegoptim gifsicle ghostscript libvips-tools webp libimage-exiftool-perl
```

**B — bundle copies into the per-user dir** (no `PATH` dependency):

```sh
scripts/fetch-tools.sh
```

**C — embed inside the executable** (single self-contained file):

```sh
scripts/fetch-tools.sh --vendor        # populates vendor/bin/<target> + current/
cargo build --release --features embed-tools
xpress bundle                          # extracts embedded binaries on demand
```

## Licensing of bundled binaries

The external tools keep their own upstream licences (several are GPL/AGPL). When
**redistributing** xpress together with them, comply with each tool's licence.
The simplest model is to ship xpress (MIT) and let users install or fetch the
tools. See [`../NOTICE.md`](../NOTICE.md).
