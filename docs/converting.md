# Converting formats

xpress converts images between **PNG, JPEG, WebP, AVIF, HEIC, GIF, TIFF and
BMP** — in any direction — and also converts audio and video. The converted
file is saved **next to the original**, which is kept.

## In the app

- **Convert everything you add:** on the Optimise screen, set **Convert to** to a
  format. Every image you drop, open or paste is then converted (videos, PDFs and
  audio are still just optimised). Set it back to *Keep format* to optimise.
- **Convert one result:** click the **format chip** on a result card
  (e.g. `PNG ▾`) and pick a format — or right-click the card → *Convert to*.

A note under the picker tells you what to expect, e.g. *transparent areas
become white* for JPEG.

## On the command line

```sh
xpress convert --to jpeg screenshot.png          # → screenshot.jpg
xpress convert --to png photo.jpg                # → photo.png (lossless)
xpress convert --to webp -r ~/Site/images        # a whole folder
xpress convert --to heic IMG_0042.jpg            # → IMG_0042.heic (macOS)
xpress convert --to jpeg IMG_0042.HEIC           # iPhone photo → JPEG
xpress convert --to webp --quality high hero.png # smallest WebP that looks "high"
```

`--to` accepts `png`, `jpeg` (or `jpg`), `webp`, `avif`, `heic`, `gif`,
`tiff` (or `tif`), `bmp` and `jxl`. `-o` picks another output file, folder or
[name template](optimising.md#output-options).

## Which format should I use?

| Format | Choose it for | Transparency | Notes |
|--------|---------------|:-----------:|-------|
| **JPEG** | Photos that must open anywhere | ✗ (becomes white) | The universal photo format. |
| **PNG** | Screenshots, graphics, anything that must stay exact | ✓ | Lossless — bigger for photos. |
| **WebP** | Websites; smaller than JPEG and PNG | ✓ | Supported by every current browser. |
| **AVIF** | The smallest files, modern browsers and apps | ✓ | Slower to encode; newer software only. |
| **HEIC** | Apple Photos, iPhone/iPad/Mac | ✓ | Writing HEIC needs macOS (Linux: `heif-enc`). |
| **GIF** | Simple graphics that must open everywhere | 1-bit (on/off) | Only 256 colours — photos get banding. |
| **TIFF** | Print, archiving, image editors | ✓ | Lossless, large. Keeps the colour profile. |
| **BMP** | Legacy Windows software | ✓ | Uncompressed — very large. |
| **JPEG XL** | Experimenting | ✓ | Needs `cjxl`; little software can open it yet. |

Rules of thumb: **screenshots → PNG or WebP**, **photos → JPEG (or WebP/AVIF for
the web)**, **iPhone photos to share → JPEG**.

## How conversions behave

- **Lossless where it can be.** PNG, TIFF and BMP keep every pixel. Converting
  to PNG is lossless by default; `--palette` makes a smaller, palette-reduced PNG
  instead (like `optimise` does).
- **Lossy formats** (JPEG, WebP, AVIF, HEIC) use the
  [compression dial](optimising.md#the-compression-dial), or — for JPEG and
  WebP — a [quality target](optimising.md#quality-targets) with `--quality`
  (in the app: *Quality target* in Preferences). If lossy WebP can't reach a
  target, a lossless WebP is used when that's still smaller than the source.
- **Transparency**: kept by every format except JPEG, where transparent areas
  are filled with white. GIF keeps it as on/off (no soft edges).
- **Orientation** is applied, so photos stay upright in every program.
- **Metadata** (EXIF, colour profile, XMP) is carried over to JPEG, PNG and
  WebP; TIFF keeps the colour profile. AVIF can't store a colour profile, so its
  pixels are converted to standard sRGB. GIF and BMP carry no metadata. Use
  `--strip-metadata` or `--strip-location` to drop it — see
  [Sharing and privacy](sharing-and-privacy.md).
- **Where the file goes**: next to the original with the new extension; the
  original is kept. Converting to the same extension replaces the file (with a
  backup).
- **Animated images** (animated GIF/WebP, APNG) aren't converted to still
  formats — that would keep only the first frame — and xpress stops with a
  message instead.

### Reading HEIC and AVIF

- **HEIC/HEIF** (iPhone photos) are read with macOS's built-in `sips`, so this
  works on macOS only.
- **AVIF** is read with `sips` on macOS and with `ffmpeg` on Linux.

Both can also be cropped, resized and used with quality targets.

## Audio and video

The same command converts audio and video:

```sh
xpress convert --to mp3 interview.wav     # audio: aac, mp3, opus, wav, flac, aiff
xpress convert --to hevc clip.mov         # video: mp4 (H.264), hevc, av1, webm
xpress convert --to gif screen.mov        # video → animated GIF
```

The source is kept. See [Video and audio](video-and-audio.md) for the details.

## Format support at a glance

See [Supported formats](formats.md) for the full table of what can be read,
written and optimised.
