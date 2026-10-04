# PDFs

## Making PDFs smaller

```sh
xpress optimise report.pdf
xpress optimise --pdf-dpi 150 -r ~/Documents/Scans
```

xpress shrinks a PDF without touching its text, vector graphics or layout:

- **Photos inside the PDF** (JPEG images) are recompressed at the JPEG quality of
  the [compression dial](optimising.md#the-compression-dial) — only if that
  makes them smaller.
- Everything else is **compressed losslessly**, and unused objects are dropped.
- As always, the original is kept as a backup and a PDF is never replaced by a
  bigger one.

Big savings usually come from scanned or photo-heavy PDFs; a PDF of mostly text
may already be as small as it gets.

### Downsampling images: `--pdf-dpi`

Scans and exported slides often contain images at far higher resolution than
they're shown. `--pdf-dpi <dpi>` scales each embedded photo down so it's no
sharper than that DPI **at the size it's drawn on the page**:

| DPI | Good for |
|----:|----------|
| 300 | Print |
| 150 | Reading on screen, email |
| 96–72 | Smallest; previews |

Images already below the limit are left alone. Accepted range: 36–600.

### What isn't touched

So that colours never change, only images that can be re-encoded exactly are
recompressed: RGB, greyscale and colour-profiled photos. CMYK (print) images,
Lab, indexed and spot-colour images are copied unchanged. Non-JPEG images
(e.g. PNG-style images inside the PDF) are compressed losslessly only.

## Cropping pages (non-destructive)

`crop-pdf` changes the visible area of every page to an aspect ratio by setting
the page's *crop box* — the content itself isn't removed, so it can be undone:

```sh
xpress crop-pdf --ratio 16:9 slides.pdf            # in place
xpress crop-pdf --ratio 4:3 --suffix -4x3 slides.pdf  # → slides-4x3.pdf
xpress uncrop-pdf slides.pdf                       # undo
```

## Pages as images

```sh
xpress extract-pages document.pdf                    # PNGs next to the PDF, 150 DPI
xpress extract-pages --format jpeg --dpi 300 -o pages/ document.pdf
```

Files are named `document-001.png`, `document-002.png`, … This command needs
**Ghostscript** (`brew install ghostscript`).
