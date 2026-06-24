# vendor/bin

Place platform-specific optimisation binaries here, one directory per Rust target
triple, e.g.:

    vendor/bin/aarch64-apple-darwin/ffmpeg
    vendor/bin/aarch64-apple-darwin/pngquant
    vendor/bin/x86_64-unknown-linux-gnu/ffmpeg

`current/` must point at (or contain a copy of) the binaries for the target you
are building for. `scripts/fetch-tools.sh` populates these directories and links
`current` to the host target.

These binaries are NOT committed and NOT part of xpress's MIT-licensed source —
each keeps its own upstream licence (see ../../NOTICE.md).
