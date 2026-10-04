# Images for the web

`xpress web` turns one image into everything a web page needs: several widths
in modern formats plus a fallback, and a ready-to-paste `<picture>` element.

```sh
xpress web hero.jpg
```

```text
✅ hero.jpg (1.4 MB) → hero-web
    hero-640.avif     640×427    18.2 KB
    hero-640.webp     640×427    31.0 KB
    hero-640.jpg      640×427    44.8 KB
    hero-1024.avif   1024×683    35.6 KB
    …
```

```html
<picture>
  <source type="image/avif" srcset="hero-640.avif 640w, hero-1024.avif 1024w, hero-1600.avif 1600w" sizes="100vw">
  <source type="image/webp" srcset="hero-640.webp 640w, hero-1024.webp 1024w, hero-1600.webp 1600w" sizes="100vw">
  <img src="hero-1600.jpg" srcset="hero-640.jpg 640w, hero-1024.jpg 1024w, hero-1600.jpg 1600w" sizes="100vw"
       width="1600" height="1067" alt="" loading="lazy" decoding="async">
</picture>
```

The snippet is printed and saved as `hero.html` with the images, in
`hero-web/` next to the source (or the folder given with `-o`).

## How the browser uses it

The browser picks the **first format it supports** (AVIF, then WebP, then the
`<img>` fallback) and, from that format, the **smallest width** that's sharp
enough for the space given by `sizes` and the screen's pixel density. Setting
`width` and `height` lets the page reserve the space, so nothing jumps while
images load.

## Options

| Option | Default | Meaning |
|--------|---------|---------|
| `--widths` | `640,1024,1600,2048` | Widths to make, comma separated. Wider than the source is never made; a source narrower than all of them gets one image at its own width. |
| `--formats` | `avif,webp` | Modern formats offered before the fallback, in order. |
| `--quality` | `high` | How good JPEG/PNG/WebP variants must look ([quality targets](optimising.md#quality-targets)); AVIF uses the compression dial. |
| `--sizes` | `100vw` | How wide the image is shown, e.g. `(max-width: 900px) 100vw, 900px`. |
| `--alt` | (empty) | The alt text — describe the image for people who can't see it. |
| `-o, --output` | `<name>-web/` | Output folder. |

The fallback is **JPEG**, or **PNG** for images with transparency.

## Example: a blog's hero image

```sh
xpress web --widths 480,960,1440 --sizes "(max-width: 960px) 100vw, 960px" \
  --alt "Sunset over the fjord" -o public/img/fjord fjord.jpg
```

Then paste the snippet from `public/img/fjord/fjord.html` into your page,
adjusting the paths in `srcset`/`src` to where the folder is served from.

## Related

- Keep a repository's images in check with [`xpress check`](ci.md).
- Convert single files with [`xpress convert`](converting.md).
