# vendor/bin

Place platform-specific optimisation binaries here, one directory per Rust target
triple, e.g.:

    vendor/bin/aarch64-apple-darwin/ffmpeg
    vendor/bin/aarch64-apple-darwin/gifski
    vendor/bin/x86_64-unknown-linux-gnu/ffmpeg

`current/` must contain a copy of the binaries for the target you are building
for (it is what `--features embed-tools` embeds). `scripts/fetch-tools.sh
--vendor` populates `<target>/` and copies it into `current/`.

These binaries are NOT committed and NOT part of xpress's MIT-licensed source —
each keeps its own upstream licence (see ../../NOTICE.md).
