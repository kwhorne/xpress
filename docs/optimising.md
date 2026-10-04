# Optimising

Optimising makes a file smaller **in the same format**. This page explains how
much xpress compresses, the three ways to tell it what you want, and how it
keeps your files safe.

```sh
xpress optimise photo.jpg                 # one file
xpress optimise -r ~/Pictures/Export      # a folder and its subfolders
xpress optimise --kind image -r ~/Desktop # only images
```

In the app, drag files onto the window — see [The desktop app](gui.md).

## Three ways to say what you want

| You want… | Use | Example |
|-----------|-----|---------|
| A sensible default | nothing (compression 30) | `xpress optimise photo.jpg` |
| A fixed amount of compression | `--compression 5..100` or `--aggressive` | `xpress optimise --aggressive *.png` |
| The smallest file that still **looks** right | `--quality` | `xpress optimise --quality high photo.jpg` |
| A file **under a size** | `--max-size` | `xpress optimise --max-size 2mb talk.mov` |
| A file that fits **a destination** | `--for` | `xpress optimise --for discord clip.mov` — see [Sharing](sharing-and-privacy.md) |

### The compression dial

One number from **5** (best quality, biggest) to **100** (smallest) controls every
format. **30** is the default; `--aggressive` means **64**. Here is what the
dial means for each encoder:

| Dial | JPEG quality | PNG | WebP / AVIF / HEIC quality | Video (H.264 CRF, preset) | Audio bitrate |
|-----:|:-----------:|-----|:-------------------------:|---------------------------|---------------|
| 5 | 98 | up to 256 colours | 89 | 18, veryfast | MP3 320 / AAC 256 / Opus 160 kbit/s |
| 15 | 97 | up to 256 colours | 78 | 19, veryfast | MP3 288 / AAC 240 / Opus 144 kbit/s |
| **30** (default) | 85 | up to 256 colours | 60 | 21, fast | MP3 256 / AAC 208 / Opus 128 kbit/s |
| 50 | 69 | up to 201 colours | 50 | 24, medium | MP3 192 / AAC 160 / Opus 96 kbit/s |
| **64** (aggressive) | 58 | up to 163 colours | 43 | 25, slow | MP3 160 / AAC 128 / Opus 80 kbit/s |
| 80 | 46 | up to 119 colours | 35 | 27, slow | MP3 112 / AAC 96 / Opus 64 kbit/s |
| 100 | 30 | up to 64 colours | 25 | 30, slower | MP3 64 / AAC 48 / Opus 32 kbit/s |

What happens per type:

- **JPEG** is re-encoded with mozjpeg (progressive, optimised), keeping
  orientation, colour profile and metadata. At quality 90 and above colour
  detail is kept at full resolution.
- **PNG** is reduced to a palette of at most the listed colours and squeezed
  losslessly. If the palette would visibly hurt the image (a quality floor that
  relaxes as the dial goes up), the PNG is kept **lossless** instead.
- **WebP** is re-encoded lossily; **HEIC** with macOS's encoder.
- **GIF**: still GIFs are re-encoded; **animated GIFs keep every frame** and
  are optimised with `gifsicle` if it's installed (otherwise left alone).
  Animated WebP and APNG are left alone.
- **Video** is re-encoded to H.264 MP4 (a `.mov` becomes `.mp4`), 8-bit 4:2:0 so
  it plays everywhere; HDR footage is tone-mapped to normal range. See
  [Video and audio](video-and-audio.md).
- **Audio** is re-encoded in its own format at the listed bitrate. FLAC is
  recompressed losslessly; WAV and AIFF are rewritten as 16-bit PCM (and WAV as
  compact ADPCM with `--aggressive`). See [Video and audio](video-and-audio.md).
- **PDF**: embedded JPEG photos are recompressed at the JPEG quality above and
  the rest is compressed losslessly. See [PDFs](pdf.md).

### Quality targets

Rather than picking a number, ask for **how good the result must look**, and
xpress finds the **smallest** file that still does:

```sh
xpress optimise --quality high photo.jpg
xpress optimise --quality visually-lossless -r ~/Screenshots
```

| Target | Score | Looks |
|--------|------:|-------|
| `visually-lossless` | 90 | indistinguishable from the original at normal size |
| `high` | 80 | excellent; differences only side by side |
| `medium` | 70 | good; fine for web and chat |
| `low` | 50 | clearly compressed, small |
| a number `1`–`100` | that | — |

The score is **SSIMULACRA2**, a measure designed to match how people judge
image quality. xpress tries several compression levels, decodes each candidate,
compares it with the original, and keeps the strongest compression that still
meets the target. The score it reached is shown on the result
(`[SSIMULACRA2 80.8]`, or `"ssimulacra2"` in `--json`).

Good to know:

- It works for **images** (JPEG, PNG and WebP output); other files in the same
  run are optimised normally.
- It never makes a file bigger: if the original already beats the target at a
  smaller size, it is kept.
- It's slower than a fixed level (roughly seven trial encodes per image).
- In the app, set *Quality target* in Preferences.

### Size budgets

`--max-size` makes each file fit under a size (decimal units: `500kb` =
500,000 bytes, `1.5mb`, or plain bytes):

```sh
xpress optimise --max-size 500kb hero.jpg
xpress optimise --max-size 8mb screen-recording.mov
```

- **Video and lossy audio** are aimed at the size directly: xpress works out
  the bitrate the budget allows for the clip's length and encodes to it (video
  in two passes), lowering the resolution if the bitrate is too thin for full
  size. Results typically land at 85–97% of the budget.
- **Images and PDFs** step up the compression until they fit.
- Files already under the budget are just optimised normally.
- If a file can't get under the budget, the smallest version is kept and you
  get a warning.

### Smallest format: `--adaptive`

For images, `--adaptive` also tries **JPEG and PNG** versions and keeps whichever
is smallest (images with transparency never become JPEG). If another format
wins, it's written **next to** the original with the new extension.

## Keeping your files safe

### Backups and restore

When a file is replaced, the original is first saved next to it as a hidden
`.<name>.orig` file (e.g. `.photo.jpg.orig`).

```sh
xpress restore photo.jpg          # put the original back
xpress restore -r ~/Pictures      # every backup under a folder
xpress clean-backups -r ~/Pictures   # delete backups once you're happy
xpress optimise --no-backup ...   # don't make backups
```

### Never bigger

If the optimised file isn't smaller, the original is kept and reported as
*already optimal*. `--allow-larger` writes the result anyway. (Conversions,
crops and resizes are always written — you asked for a change.)

### Dates and metadata

The original modification date is kept (`--no-preserve-dates` to turn off).
Photo orientation, colour profile, EXIF and XMP are kept; see
[Sharing and privacy](sharing-and-privacy.md) to remove them.

### Skipping files already optimised

After optimising, xpress tags each file (an extended attribute recording the
settings and a fingerprint of the content). Next time, a file whose content is
unchanged and that was optimised at least as hard is **skipped instantly**:

```text
⚠️ photo.jpg already optimised with these settings — skipped (--force to redo)
```

This saves time and avoids losing quality to repeated re-encoding. A file runs
again if it was edited, or you ask for more compression, `--strip-metadata`,
`--strip-location` or a lower `--pdf-dpi`. `--force` always re-processes. The
tag is lost when files are copied to filesystems without extended attributes
(and by git), so a fresh checkout is processed again.

## Output options

| Option | Effect |
|--------|--------|
| `-o, --output <path>` | Write elsewhere: a file (one input), a folder (several inputs), or a template |
| `-j, --jobs <n>` | Files processed in parallel (default: one per CPU core) |
| `--timeout <secs>` | Stop any single external tool (e.g. ffmpeg) that runs longer |
| `--json` | Machine-readable results |
| `-q, --quiet` | Only errors and the summary |

**Output templates** — when `--output` contains `%` tokens it's a file name
pattern: `%f` name without extension, `%e` extension, `%P` parent folder,
`%y %m %d %H %M %S` date and time, `%i` a counter, `%r` random characters,
`%%` a literal `%`.

```sh
xpress optimise -o '~/Export/%f-small.%e' *.jpg
xpress optimise -o '%P/optimised/%f.%e' -r ~/Shots
```

With `-o` the originals are left untouched and no backups are made.

## Defaults

The defaults (compression, aggressive, backups, strip metadata, keep dates) can
be changed in the config file — see [Configuration](configuration.md). Flags
always win over the config file.
