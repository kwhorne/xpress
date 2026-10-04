# Video and audio

Video and audio are encoded with **ffmpeg**. The macOS app includes it; on the
command line, install it or point xpress at the app's copy — see
[Installation → Extra tools](installation.md#extra-tools).

## Optimising video

```sh
xpress optimise screen-recording.mov       # → screen-recording.mp4
xpress optimise --aggressive -r ~/Movies/Exports
```

- Videos are re-encoded to **H.264 in an MP4** container — the format that plays
  everywhere. A `.mov` (or `.mkv`, `.avi`, …) becomes `.mp4` and replaces the
  original, which is kept as a backup.
- How hard it compresses follows the [compression dial](optimising.md#the-compression-dial):
  H.264 CRF 18 (best) to 30 (smallest), with slower, more efficient encoder
  presets at higher settings.
- The audio track is copied unchanged when possible (otherwise re-encoded to AAC).
- The MP4 is made **streamable** (`faststart`), and always **8-bit 4:2:0**, so it
  plays in QuickTime, Safari, browsers and on phones.
- **HDR video** (e.g. iPhone footage in HLG or Dolby Vision/PQ) is
  **tone-mapped** to normal range, so it doesn't come out grey and washed out.
- A video is never replaced by a **bigger** one: if H.264 can't beat the
  original (e.g. an efficient HEVC clip), the original is kept.

## Fitting a video under a size

```sh
xpress optimise --max-size 10mb demo.mov
xpress optimise --for discord clip.mov        # 20 MB limit
```

xpress reads the clip's length, works out the bitrate the budget allows, and
encodes in **two passes** to hit it. If that bitrate is too low for the full
resolution, the video is **scaled down** (never below 240 lines) so it stays
watchable; if the first try misses, it corrects and tries again. Results land
at roughly 85–97% of the budget. See [Sharing and privacy](sharing-and-privacy.md)
for the `--for` presets.

## Converting video

```sh
xpress convert --to mp4  clip.mkv     # H.264 MP4
xpress convert --to hevc clip.mov     # HEVC (H.265) MP4 — smaller, plays on Apple devices
xpress convert --to av1  clip.mov     # AV1 MP4 — smallest, slow to encode
xpress convert --to webm clip.mov     # VP9 + Opus WebM — for the web
xpress convert --to hevc --hw clip.mov   # Apple's hardware encoder: much faster, larger files
```

- The source is kept; the result goes next to it.
- `--hw` uses Apple silicon's hardware encoder (VideoToolbox) for H.264 and HEVC.
- HEVC, AV1 and VP9 keep HDR as HDR (10-bit); only H.264 is tone-mapped.

### Video → animated GIF

```sh
xpress convert --to gif screen-capture.mov
```

Frames are taken at 15 fps. If [`gifski`](https://gif.ski) is installed it makes
the GIF (much better colours); otherwise ffmpeg does. `--aggressive` lowers the
quality for a smaller file. For a smaller GIF, crop or downscale the video first.

### Resizing and cropping video

`xpress downscale` and `xpress crop` work on videos too — see
[Resizing and cropping](resizing-and-cropping.md).

### More video edits (pipelines)

These are available as [pipeline](pipelines.md) steps:

| Step | Does |
|------|------|
| `removeAudio` | Drop the audio track (no re-encode) |
| `changeSpeed(factor: 2.0)` | Speed up / slow down (0.25–8×), audio included |
| `capFps(fps: 30)` | Limit the frame rate |
| `watermark(image: "logo.png", position: bottomRight, opacity: 0.8, scale: 0.15)` | Overlay a logo |

```sh
xpress pipeline run 'removeAudio -> changeSpeed(factor: 2.0) -> downscale(factor: 0.5)' demo.mov
```

## Audio

```sh
xpress optimise podcast.mp3                 # same format, smaller
xpress convert --to mp3 interview.wav       # → interview.mp3
xpress convert --to opus --bitrate 64 talk.m4a
xpress optimise --max-size 5mb episode.mp3  # aim for a size
```

| Format | `--to` | Extension | Notes |
|--------|--------|-----------|-------|
| AAC | `aac` | `.m4a` | Apple's encoder on macOS; ffmpeg's elsewhere |
| MP3 | `mp3` | `.mp3` | Variable bitrate; a fixed bitrate when you give one (`--bitrate`, size budgets) |
| Opus | `opus` | `.ogg` | Best quality for its size, especially speech |
| FLAC | `flac` | `.flac` | Lossless, compressed |
| WAV | `wav` | `.wav` | 16-bit PCM; `--aggressive` uses compact ADPCM |
| AIFF | `aiff` | `.aiff` | 16-bit PCM |

- **Optimising** keeps the format and lowers the bitrate according to the
  [compression dial](optimising.md#the-compression-dial). The file is never
  replaced by a bigger one.
- **Converting** writes the new format next to the source and keeps the
  original. `--bitrate <kbit/s>` sets the bitrate explicitly.
- **Loudness**: the pipeline step `normalize(lufs: -16)` evens out the volume
  (EBU R128) — e.g. `xpress pipeline run 'normalize -> convert(to: mp3)' talk.wav`.
- **Lower a bitrate**: `lowerBitrate(kbps: 96)` in a pipeline.

## Location in videos

`--strip-location` removes where a video was recorded (QuickTime/MP4 location
metadata); `--strip-metadata` removes all of a video's metadata. See
[Sharing and privacy](sharing-and-privacy.md).
