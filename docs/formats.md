# Supported formats

✓ = supported · — = not supported · notes in brackets.

## Images

| Format | Extensions | Read | Optimise | Convert to | Transparency | Metadata kept |
|--------|-----------|:----:|:--------:|:----------:|:------------:|---------------|
| JPEG | `.jpg` `.jpeg` | ✓ | ✓ (mozjpeg) | ✓ | — | EXIF, ICC, XMP |
| PNG | `.png` | ✓ | ✓ (palette + lossless) | ✓ (lossless, or `--palette`) | ✓ | ICC, EXIF, XMP |
| WebP | `.webp` | ✓ | ✓ (lossy) | ✓ | ✓ | ICC, EXIF, XMP |
| AVIF | `.avif` | ✓ (macOS: sips; Linux: ffmpeg) | ✓ | ✓ | ✓ | — (converted to sRGB) |
| HEIC / HEIF | `.heic` `.heif` | ✓ (macOS only) | ✓ (macOS only) | ✓ (macOS; Linux needs `heif-enc`) | ✓ | (handled by macOS) |
| GIF | `.gif` | ✓ | ✓ (animated: needs `gifsicle`) | ✓ (still) | 1-bit | — |
| TIFF | `.tif` `.tiff` | ✓ | ✓ | ✓ | ✓ | ICC |
| BMP | `.bmp` | ✓ | ✓ | ✓ | ✓ | — |
| JPEG XL | `.jxl` | — | — | ✓ (needs `cjxl`) | ✓ | — |

- **Animated** GIF, WebP and PNG (APNG) keep all frames when optimised (animated
  GIFs via `gifsicle`, the others are left as they are); they can't be
  converted to still formats, resized or cropped.
- Photo **orientation** is always applied, so images display upright everywhere.
- 16-bit images keep their depth in lossless PNG and TIFF output; formats that
  store 8 bits per channel (JPEG, WebP, AVIF, GIF, palette PNG) reduce them to 8.

## Video

| | Read | Output |
|--|------|--------|
| Containers | `.mov` `.mp4` `.m4v` `.mkv` `.webm` `.avi` `.mpg` `.mpeg` `.m2v` | `.mp4` (H.264, HEVC, AV1) or `.webm` (VP9) |
| Codecs | anything ffmpeg can decode | H.264 (optimise), HEVC, AV1, VP9 (convert), animated GIF |
| HDR | HLG and PQ (HDR10, Dolby Vision base layer) | H.264 output is tone-mapped to SDR; HEVC/AV1/VP9 keep HDR |

Requires ffmpeg (bundled in the app).

## Audio

| Format | Extensions | Read | Optimise | Convert to |
|--------|-----------|:----:|:--------:|:----------:|
| AAC | `.m4a` | ✓ | ✓ | ✓ |
| MP3 | `.mp3` | ✓ | ✓ | ✓ |
| Opus / Ogg | `.ogg` `.oga` | ✓ | ✓ | ✓ (`.ogg`) |
| FLAC | `.flac` | ✓ | ✓ (lossless) | ✓ |
| WAV | `.wav` | ✓ | ✓ | ✓ |
| AIFF | `.aiff` `.aif` | ✓ | ✓ | ✓ |

Requires ffmpeg (bundled in the app).

## PDF

`.pdf` — optimise (embedded photos recompressed, everything else compressed
losslessly, optional downsampling), non-destructive crop/uncrop, and page
rendering to PNG/JPEG (needs Ghostscript). See [PDFs](pdf.md).
