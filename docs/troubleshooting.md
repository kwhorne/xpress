# Troubleshooting & FAQ

## General

**Where did my original go?**
It's next to the file as a hidden `.<name>.orig`. Restore it with
`xpress restore <file>` (or a folder with `-r`). In Finder, show hidden files
with ⌘⇧. (Command-Shift-period).

**It says "already optimal" and nothing changed.**
xpress never replaces a file with a bigger or equal one. The file is already
well compressed at this setting. Try `--aggressive`, a higher `--compression`,
or a [quality target](optimising.md#quality-targets).

**It says "already optimised with these settings — skipped".**
xpress optimised this file before and it hasn't changed since. Use `--force` to
do it again, or ask for more compression. In the app: Preferences → *Skip
already-optimised files*.

**Can I undo a whole folder?**
`xpress restore -r <folder>` puts every backed-up original back.
`xpress clean-backups -r <folder>` deletes the backups once you're happy.

**The result looks worse than I'd like.**
Lower the compression (`--compression 15`) or use
`--quality visually-lossless`, which picks the smallest file that still looks
identical.

## Video and audio

**"`ffmpeg` not found".**
The command line needs ffmpeg for video and audio. Install it
(`brew install ffmpeg` / `apt install ffmpeg`) or use the app's copy:
`export XPRESS_BIN_DIR=/Applications/xpress.app/Contents/Resources/bin`.
Run `xpress doctor` to check.

**My `.mov` turned into an `.mp4`.**
Optimising re-encodes video to H.264 MP4, which plays everywhere. The `.mov` is
kept as a backup (`.<name>.mov.orig`). To change format while keeping the
original, use `xpress convert --to …` instead.

**The video got bigger / nothing happened.**
Videos that are already efficiently encoded (e.g. iPhone HEVC clips) often can't
be beaten by H.264 at the same quality; the original is kept. Use
`--max-size` or `--for` to reach a size, or `xpress convert --to hevc`.

**HDR video looks different after optimising.**
H.264 output is tone-mapped from HDR to normal range so it plays correctly
everywhere. To keep HDR, convert to HEVC: `xpress convert --to hevc clip.mov`.

**Encoding takes very long.**
High-resolution video and AV1 are slow. `--hw` (Apple silicon) uses the
hardware encoder for H.264/HEVC conversions; `--timeout <secs>` stops runaway
jobs.

## Images

**My animated GIF wasn't made smaller.**
Animated GIFs are optimised with `gifsicle` — install it
(`brew install gifsicle`). Without it they're left untouched, never flattened.

**Converting an animated GIF to WebP/PNG fails.**
That would keep only the first frame, so xpress refuses. For animation, convert
the source video instead (`xpress convert --to gif video.mov`).

**HEIC doesn't work on Linux.**
Reading HEIC uses macOS's built-in tools. On Linux, convert on a Mac or with
another tool first; writing HEIC on Linux needs `heif-enc`.

**Colours look different after converting to AVIF.**
AVIF files from xpress can't carry a colour profile, so wide-gamut (Display P3)
images are converted to sRGB. The most saturated colours may be slightly less
vivid; use WebP or JPEG to keep the profile.

**Transparent areas turned white.**
JPEG has no transparency. Use PNG, WebP or AVIF for images with transparency.

## The app

**I can't find the window.**
xpress lives in the menu bar (✕ icon), not the Dock. Click it or press ⌘⇧X.
Closing the window only hides it.

**The hotkeys don't work.**
Another app may already use ⌘⇧O or ⌘⇧X. Quit the other app or use the menu-bar
menu.

**My Preferences were reset.**
App settings currently last until you quit; they aren't saved between launches
yet.

**Update & Restart failed.**
Download the latest `.dmg` from the
[releases page](https://github.com/kwhorne/xpress/releases/latest) and replace
the app.

## Command line

**"no optimisable files found".**
The paths contain no supported files (see [Supported formats](formats.md)), or
you passed a folder without `-r` and the files are in subfolders.

**`xpress update` fails.**
Download the `.tar.gz` from the releases page and replace the binary. (Versions
before 0.5.0 on macOS could pick the wrong download; updating to 0.5.0 or later
fixes this.) Versions before 0.5.3 fail with `status code 403` when GitHub's
API rate limit (60 requests an hour per network address) is reached: wait an
hour, or download manually. Later versions fall back to the releases page.

**"checksum mismatch".**
The download didn't match the release's published SHA-256 checksum, so nothing
was replaced. Try again; if it keeps happening, download manually and
[report it](https://github.com/kwhorne/xpress/issues).

**Scripting.**
`--json` prints machine-readable results; `-q` prints only errors and the
summary; `xpress check` exits with status 1 when files need work.

## Still stuck?

Open an issue at [github.com/kwhorne/xpress/issues](https://github.com/kwhorne/xpress/issues)
with the command, the output of `xpress doctor`, and your xpress version
(`xpress --version`).
