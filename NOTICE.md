# NOTICE

## About this project

**xpress** is an independent media-optimisation tool written from scratch in Rust
and released under the [MIT License](LICENSE).

xpress is **not a fork of Clop** and does **not** contain Clop source code. It is
inspired by the *functionality and ideas* of [Clop](https://lowtechguys.com/clop)
(an image/video/PDF/clipboard optimiser). Functionality and ideas are not subject
to copyright; xpress reimplements that behaviour independently, deriving its
command invocations and parameters from the public documentation of the
underlying tools (ffmpeg, pngquant, jpegoptim, gifsicle, ghostscript, etc.).

If you want the original Clop application, get it at https://lowtechguys.com/clop —
it is a separate work under its own (GPLv3) licence.

## Bundled third-party binaries

xpress can bundle or download external command-line tools to perform the actual
encoding/optimisation. These tools are **separate programs with their own
licences** and are not part of xpress's source. They are invoked as subprocesses,
not linked into xpress. Each retains its original licence:

| Tool        | Purpose                  | Typical licence        |
|-------------|--------------------------|------------------------|
| ffmpeg      | video / audio            | LGPL-2.1+ / GPL-2.0+   |
| pngquant    | PNG compression          | GPL-3.0 / commercial   |
| jpegoptim   | JPEG optimisation        | GPL-2.0+               |
| gifsicle    | GIF optimisation         | GPL-2.0+               |
| ghostscript | PDF optimisation         | AGPL-3.0 / commercial  |
| libvips     | image resizing           | LGPL-2.1+              |
| gifski      | video → GIF              | AGPL-3.0               |
| cwebp/webp  | WebP encoding            | BSD-3-Clause           |
| libheif/x265| HEIC/AVIF encoding       | LGPL / GPL             |
| exiftool    | metadata                 | Perl (Artistic/GPL)    |

When **redistributing** xpress together with any of these binaries, you must
comply with each binary's licence (which may, for the GPL/AGPL tools, require
making their corresponding source available). The simplest distribution model is
to ship xpress under MIT and let users install or fetch the tools themselves
(`xpress doctor`, `scripts/fetch-tools.sh`), keeping xpress's own redistribution
unencumbered.
