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
Another app may already use ⌘⇧O, ⌘⇧X or ⌃⌘V — Preferences → Shortcuts says so
when one couldn't be set up. Pick different keys there, or use the menu-bar
menu.

**The clipboard history doesn't record anything.**
Turn it on under *History* or in Preferences — it's off by default. Only what
you copy afterwards is recorded, and never content a password manager marks as
private. Screenshots are picked up from the folder macOS saves them to (set in
the Screenshot app's *Options*); it can take a few seconds after the floating
thumbnail disappears.

**Paste directly doesn't paste.**
xpress needs the Accessibility permission: System Settings → Privacy & Security
→ Accessibility → switch on *xpress* (remove and re-add it if it's listed but
still doesn't work). xpress presses the key that gives ⌘V in your current
keyboard layout (Dvorak, AZERTY and “⌘ QWERTY” layouts included).

**Only the first part of a multi-clip is pasted.**
Some apps take only one kind of content. Plain text fields get all the text;
rich editors get text and images; a pure picture field gets the first image.
Right-click the multi-clip → *Show items* to copy items one by one.

**There's no Apple Intelligence menu.**
It needs macOS 26 or later on Apple silicon, Apple Intelligence turned on
(System Settings → Apple Intelligence & Siri), and a text clip (or an image with
recognised text). A greyed-out item tells you what's missing; the model may
still be downloading after you turn it on.

**History doesn't sync.**
Turn on *Sync with iCloud* on **every** Mac, and check that iCloud Drive is on.
Preferences shows the last sync and any problem. Images can take a while to
reach the other Mac (“changes waiting for iCloud”); with *Optimise Mac Storage*
on, iCloud may first need to download them.

**A screenshot isn't found by its text.**
Text recognition needs *Find text in images* on, runs once when the image is
recorded, and reads printed text best; tiny or handwritten text may be missed.

**My Preferences were reset.**
Versions before 0.5.4 didn't save app settings between launches. Later versions
keep them in `gui.json` in the config folder (see
[Configuration](configuration.md#desktop-app-settings-guijson)); if that file
is unreadable, the app starts from the defaults again.

**Dropped images are converted instead of optimised.**
*Convert to* on the Optimise screen is remembered between launches. Set it back
to *Keep format*.

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
