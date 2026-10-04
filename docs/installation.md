# Installation

## macOS app (recommended)

1. Download `xpress-<version>-macos-aarch64-apple-darwin.dmg` from the
   [latest release](https://github.com/kwhorne/xpress/releases/latest).
2. Open it and drag **xpress** to *Applications*.
3. Start it from *Applications* or Spotlight. It runs as a **menu-bar app**
   (no Dock icon); closing the window hides it to the menu bar.

The app is Developer ID-signed and notarised, needs **Apple silicon**
(M1 or later), and is self-contained: `ffmpeg` for video and audio is bundled.
A `-app.zip` with the same app is also published if you prefer a zip.

## Command line

The `xpress` command line tool is a separate download — the `.tar.gz` for your
platform on the [releases page](https://github.com/kwhorne/xpress/releases/latest):

| Platform | File |
|----------|------|
| macOS, Apple silicon | `xpress-<version>-aarch64-apple-darwin.tar.gz` |
| Linux, x86-64 | `xpress-<version>-x86_64-unknown-linux-gnu.tar.gz` |

```sh
tar xzf xpress-v0.7.1-aarch64-apple-darwin.tar.gz
sudo mv xpress-v0.7.1-aarch64-apple-darwin/xpress /usr/local/bin/
xpress --version
```

Each archive also contains `xpress-gui` (the desktop app as a plain binary),
plus the README, licence and notice. Every download has a matching `.sha256`
file: `shasum -a 256 -c xpress-…tar.gz.sha256`.

### Extra tools

Everything for **images and PDFs** is built in. A few features use external
programs, which xpress looks for automatically:

| Tool | Needed for | Without it |
|------|------------|------------|
| `ffmpeg` | all video and audio | video/audio files fail with "ffmpeg not found" |
| `gifsicle` | optimising **animated** GIFs | animated GIFs are left as they are |
| `gifski` | best-quality video → GIF | ffmpeg makes the GIF instead |
| `ghostscript` (`gs`) | `xpress extract-pages` | that command fails |
| `exiftool` | `xpress strip-exif` and the `stripExif` pipeline step | those fail (use `--strip-metadata` instead) |
| `heif-enc` | HEIC output on Linux | (macOS uses its built-in encoder) |
| `cjxl` | JPEG XL output | JPEG XL conversion fails |

The desktop app bundles `ffmpeg`. **Command-line users** can either install
ffmpeg (`brew install ffmpeg`, `apt install ffmpeg`) or reuse the app's copy:

```sh
export XPRESS_BIN_DIR="/Applications/xpress.app/Contents/Resources/bin"
```

Check what xpress can find with:

```sh
xpress doctor
```

xpress looks for each tool in this order: `$XPRESS_BIN_DIR`, a `bin/` folder
next to the `xpress` executable, the per-user tools folder
(`~/Library/Application Support/xpress/bin` on macOS,
`~/.local/share/xpress/bin` on Linux), then your `PATH`. A portable ffmpeg can
be put in the per-user folder with
`scripts/fetch-static-tools.sh <target> "<that folder>"` from the source tree.

## Updating

- **App:** when a new version is out, a banner offers **Update & Restart**. You
  can also choose *Check for updates* from the menu-bar icon or the About page.
  The app checks every six hours.
- **Command line:** `xpress update` downloads and installs the new version in
  place; `xpress update --check` only reports.

Downloads are checked against the release's SHA-256 checksums before anything
is replaced. The update check uses GitHub's API (60 anonymous requests an hour
per network address) and falls back to the public releases page when that
limit is reached; set `GITHUB_TOKEN` to use your own quota.

## Shell completions and man page

```sh
xpress completions zsh  > ~/.zfunc/_xpress                     # also bash, fish, powershell, elvish
xpress completions fish > ~/.config/fish/completions/xpress.fish
xpress man | sudo tee /usr/local/share/man/man1/xpress.1 >/dev/null
```

## Uninstalling

- Quit xpress from its menu-bar icon and move `xpress.app` to the Bin.
- Remove the command line tool: `sudo rm /usr/local/bin/xpress`.
- Optional: delete settings and saved pipelines in
  `~/Library/Application Support/xpress` (Linux: `~/.config/xpress` and
  `~/.local/share/xpress`), and optimised clipboard images in `~/Pictures/xpress`.
- Backups (`.<name>.orig`) stay next to your files until you remove them
  (`xpress clean-backups -r <folder>`).

## Building from source

Requires Rust 1.90 or newer.

```sh
git clone https://github.com/kwhorne/xpress.git
cd xpress
cargo build --release          # target/release/xpress and target/release/xpress-gui
cargo install --path crates/xpress-cli
```

On Linux the desktop app needs GTK and X11 development packages
(`libgtk-3-dev libxdo-dev libxcb-render0-dev libxcb-shape0-dev
libxcb-xfixes0-dev libxkbcommon-dev`). Building the macOS `.app`/`.dmg` and
signing are covered in [The desktop app](gui.md#building-the-app) and
[Code signing](signing.md).

## Licences of external tools

The tools above are separate programs with their own licences (some are
GPL). xpress itself only links permissively licensed libraries; see
[`NOTICE.md`](../NOTICE.md).
