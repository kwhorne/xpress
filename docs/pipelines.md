# Pipeline DSL

A pipeline is a chain of **steps** joined by `->`. Steps run left to right; each
transforms the working file and feeds the next.

```text
crop(width: 1600) -> convert(to: webp) -> downscale(factor: 0.5)
```

Run one inline, or save it and run by name:

```sh
xpress pipeline run 'crop(width: 1600) -> convert(to: webp)' photo.png
xpress pipeline add web 'crop(longEdge: 2000) -> convert(to: webp)'
xpress pipeline run web *.png
```

## Steps

| Step | Parameters | Applies to | Notes |
|------|------------|-----------|-------|
| `optimise` | — | all | Optimise in place using the current compression value. |
| `downscale` | `factor:` (`0.5` or `50%`) | image, video | Scale keeping aspect ratio. |
| `crop` | `width:`, `height:`, `longEdge:`, `ratio:` (`16:9`), `smart:` | image, video | Resize/crop to a size or aspect ratio. |
| `convert` | `to:` image/audio format, or video target (`gif`, `mp4`, `hevc`, `av1`, `webm`) | image, audio, video | Changes the file type. |
| `adaptive` | — | image | Try multiple formats, keep the smallest (alpha-aware). |
| `targetSize` | `bytes:` (`500kb`, `1.5mb`) | all | Compress to fit a byte budget. |
| `normalize` | `lufs:` (default −16) | audio | Loudness-normalise (EBU R128). |
| `watermark` | `image:`, `position:`, `opacity:`, `scale:` | image, video | Overlay a watermark. |
| `copyToClipboard` | — | image | Copy the result to the clipboard (macOS). |
| `runScript` | `code:` or `path:` | all | Run a shell script (`$FILE` = current file). |
| `stripExif` | — | image | Remove metadata. |
| `removeAudio` | — | video | Drop the audio track. |
| `changeSpeed` | `factor:` | video | `2.0` = twice as fast. |
| `capFps` | `fps:` | video | Limit frame rate. |
| `lowerBitrate` | `kbps:` | audio | Re-encode at a lower bitrate. |

### Parameter syntax

- Numbers: `width: 1600`, `fps: 30`.
- Factors: a decimal `0.5` or a percentage `50%`.
- Ratios: `ratio: 16:9`.
- Strings: bare (`to: webp`) or quoted (`to: "webp"`).
- Booleans: `smart: true`.

## Output placement

- If `--output` is given, the final artifact goes there.
- Otherwise, if the type is unchanged it **replaces** the source (with a
  `.<name>.orig` backup unless `--no-backup`).
- If a step changed the type (e.g. `convert`), the result is written **alongside**
  the source with the new extension, keeping the original.

## Managing pipelines

```sh
xpress pipeline list           # saved pipelines + folder automations
xpress pipeline show web       # parsed steps + canonical DSL
xpress pipeline delete web
```

Saved pipelines and automations live in a JSON file in the per-user config dir
(`~/Library/Application Support/xpress/pipelines.json`, or the XDG/`APPDATA`
equivalent).

## Examples

```text
# Shrink screenshots to web-ready WebP under a long edge
crop(longEdge: 2000) -> convert(to: webp)

# Make a fast, audio-free preview clip
removeAudio -> changeSpeed(factor: 2.0) -> downscale(factor: 0.5)

# Strip metadata then optimise hard
stripExif -> optimise

# Re-encode podcasts smaller
convert(to: mp3) -> lowerBitrate(kbps: 96)

# Turn a screen recording into a shareable GIF
convert(to: gif)

# Get any image under 300 KB, smallest format wins
adaptive -> targetSize(bytes: 300kb)
```
