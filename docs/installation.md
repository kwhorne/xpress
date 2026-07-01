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

**Images and PDFs are optimised, resized, cropped and converted entirely in pure
Rust — no external tool required.** The macOS app also **bundles `ffmpeg`** for
video/audio, so a released `.app`/`.dmg` needs nothing installed. External
binaries are only relevant when building from source or for a few extras:

| Tool          | Used for                         |
|---------------|----------------------------------|
| `ffmpeg`      | video and audio (bundled in the app) |
| `ghostscript` (`gs`) | `extract-pages` only (optional) |
| `gifski`      | video → GIF (optional; ffmpeg fallback) |
| `heif-enc`    | HEIC conversion (optional)       |
| `cjxl`        | JPEG XL conversion (optional)    |
| `exiftool`    | metadata (optional)              |

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

## Shell completions & man page

The CLI can emit its own completions and a man page:

```sh
# Completions (bash, zsh, fish, powershell, elvish)
xpress completions zsh  > ~/.zfunc/_xpress
xpress completions bash > /usr/local/etc/bash_completion.d/xpress
xpress completions fish > ~/.config/fish/completions/xpress.fish

# Man page
xpress man > /usr/local/share/man/man1/xpress.1
```

## Licensing of bundled binaries

The external tools keep their own upstream licences (several are GPL/AGPL). When
**redistributing** xpress together with them, comply with each tool's licence.
The simplest model is to ship xpress (MIT) and let users install or fetch the
tools. See [`../NOTICE.md`](../NOTICE.md).
