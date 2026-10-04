# Pipelines

A pipeline is a chain of **steps** joined by `->`. Each step works on the
result of the previous one:

```text
crop(longEdge: 2000) -> convert(to: webp)
```

Run one directly, or save it under a name:

```sh
xpress pipeline run 'crop(longEdge: 2000) -> convert(to: webp)' photo.png
xpress pipeline add web 'crop(longEdge: 2000) -> convert(to: webp)'
xpress pipeline run web -r ~/Export
```

Pipelines also power [folder automations](daemon.md) and the *Pipeline* switch
in the [desktop app](gui.md). The [common options](cli.md#common-options)
(`--compression`, `--strip-location`, `-o`, …) apply to every step.

## Steps

| Step | Parameters | Works on | Does |
|------|------------|----------|------|
| `optimise` | — | everything | Optimise using the compression dial |
| `adaptive` | — | images | Try JPEG/PNG too, keep the smallest |
| `targetSize` | `bytes:` (`500kb`, `1.5mb`, or bytes) | everything | Fit under a size (video/audio by bitrate) |
| `convert` | `to:` | images, audio, video | Change format: any image format (`png`, `jpeg`, `webp`, `avif`, `heic`, `gif`, `tiff`, `bmp`, `jxl`), audio (`aac`, `mp3`, `opus`, `wav`, `flac`, `aiff`), video (`mp4`, `hevc`, `av1`, `webm`, `gif`) |
| `downscale` | `factor:` (`0.5` or `50%`) | images, video | Scale down, keeping the aspect ratio |
| `crop` | `width:`, `height:`, `longEdge:`, `ratio:` (`16:9`), `smart: true` | images, video | Resize/crop — same rules as [`xpress crop`](resizing-and-cropping.md) |
| `watermark` | `image:`, `position:`, `opacity:`, `scale:` | images, video | Overlay a logo (see below) |
| `stripExif` | — | images | Remove metadata without re-encoding (needs `exiftool`) |
| `removeAudio` | — | video | Drop the audio track |
| `changeSpeed` | `factor:` (0.25–8) | video | `2.0` = twice as fast; audio follows |
| `capFps` | `fps:` | video | Limit the frame rate |
| `lowerBitrate` | `kbps:` | audio | Re-encode at this bitrate |
| `normalize` | `lufs:` (default `-16`) | audio | Even out loudness (EBU R128) |
| `copyToClipboard` | — | PNG images | Put the result on the clipboard (macOS) |
| `runScript` | `code:` or `path:` | everything | Run a shell script on the current file |

**watermark** — `image:` is the path to the logo, absolute or relative to the
current folder (a PNG with transparency works best); `position:` is `topLeft`, `topRight`, `bottomLeft`, `bottomRight`
(default) or `center`; `opacity:` 0–1 (default 1); `scale:` the logo's width as
a fraction of the picture's (default 0.15).

**runScript** — `code:` runs a shell command with the current file in `$FILE`;
`path:` runs a script file, which gets the current file as its first argument
(and in `$FILE`). The file passes through unchanged, so put `runScript` last to
act on the final result.

### Syntax

- Parameters: `name: value`, separated by commas — `crop(width: 1600, height: 900)`.
- Numbers `1600`; factors `0.5` or `50%`; ratios `16:9`; booleans `true`.
- Text may be quoted: `to: webp` or `to: "webp"`; quote paths and code.
- Steps without parameters can drop the parentheses: `optimise`, `adaptive`.
- Some names also accept `snake_case`: `long_edge`, `strip_exif`,
  `remove_audio`, `change_speed`, `cap_fps`, `lower_bitrate`, `target_size`,
  `copy_to_clipboard`, `run_script`; `aspect:` works like `ratio:`, `kb:` like
  `bytes:`.

## Where the result goes

- With `-o`, the final file goes there and the source is untouched.
- If the final file has the **same type** as the source, it replaces it, with a
  `.<name>.orig` backup (unless `--no-backup`).
- If a step changed the type (e.g. `convert`), the result is written **next to**
  the source with the new extension; the source is kept.
- A pipeline that only tries to shrink a file (`optimise`, `adaptive`,
  `stripExif`, `lowerBitrate`, `targetSize`) never replaces it with a bigger
  one. Pipelines that change content (crop, convert, watermark, …) are always
  applied.

## Managing saved pipelines

```sh
xpress pipeline list           # saved pipelines and folder automations
xpress pipeline show web       # the steps of one pipeline
xpress pipeline delete web
```

They're stored in `pipelines.json` in the config folder — see
[Configuration](configuration.md).

## Examples

```text
# Web-ready screenshots
crop(longEdge: 2000) -> convert(to: webp)

# Social card from any image, keeping the interesting part
crop(width: 1200, height: 630, smart: true) -> convert(to: jpeg)

# Watermark product photos
crop(longEdge: 1600) -> watermark(image: "brand/logo.png", position: bottomRight, opacity: 0.8)

# A quick, silent preview clip
removeAudio -> changeSpeed(factor: 2.0) -> downscale(factor: 0.5)

# Podcast: even loudness, smaller MP3
normalize(lufs: -16) -> convert(to: mp3) -> lowerBitrate(kbps: 96)

# Screen recording → GIF
downscale(factor: 0.5) -> convert(to: gif)

# Any image under 300 KB
adaptive -> targetSize(bytes: 300kb)

# Optimise, then upload and copy the link (your own uploader)
convert(to: webp) -> runScript(code: "url=$(myuploader \"$FILE\") && printf %s \"$url\" | pbcopy")
```
