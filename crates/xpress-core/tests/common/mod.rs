//! Shared test harness: installs stub optimisation tools into a temp bin dir and
//! provides sample media files. The stubs are deterministic (they shrink files)
//! so the engine's plumbing — argument building, temp handling, backups, size
//! guard, placement — can be tested without real codecs.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// A tiny but valid 4x4 PNG (so `imagesize` can read real dimensions).
const PNG_4X4_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAEUlEQVR4nGNkYPhfz0BHwDhqGAMABzMCpQ4P0XwAAAAASUVORK5CYII=";

fn stub(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/bash\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
    }
}

/// Install the stub tool set once and point xpress-core's resolver at it.
pub fn install_stubs() {
    static BIN_DIR: OnceLock<PathBuf> = OnceLock::new();
    let dir = BIN_DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("xpress-stubs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // halve <src> into <out>
        let halve = r#"sz=$(wc -c < "$src"); head -c $((sz/2 + 1)) "$src" > "$out""#;

        // pngquant --force --speed N --quality Q --output <out> <src>
        stub(&dir, "pngquant", &format!(
            r#"out=""; args=("$@"); src="${{args[${{#args[@]}}-1]}}"
for ((i=0;i<${{#args[@]}};i++)); do [[ "${{args[$i]}}" == "--output" ]] && out="${{args[$((i+1))]}}"; done
{halve}"#));

        // jpegoptim ... --dest <dir> <file>  (in-place safe: buffer via temp)
        let jpeg_body = r#"dest=""; args=("$@"); src="${args[${#args[@]}-1]}"
for ((i=0;i<${#args[@]};i++)); do [[ "${args[$i]}" == "--dest" ]] && dest="${args[$((i+1))]}"; done
out="$dest/$(basename "$src")"
tmp=$(mktemp); sz=$(wc -c < "$src"); head -c $((sz/2 + 1)) "$src" > "$tmp"; mv "$tmp" "$out""#;
        stub(&dir, "jpegoptim", jpeg_body);
        stub(&dir, "jpegoptim-old", jpeg_body);

        // gifsicle [opts] --output <out> <src>   (also handles --scale resize)
        stub(&dir, "gifsicle", &format!(
            r#"out=""; args=("$@"); src="${{args[${{#args[@]}}-1]}}"
for ((i=0;i<${{#args[@]}};i++)); do [[ "${{args[$i]}}" == "--output" ]] && out="${{args[$((i+1))]}}"; done
{halve}"#));

        // gs ... -o <out> ... <input.pdf>
        stub(&dir, "gs", &format!(
            r#"out=""; args=("$@"); src=""
for ((i=0;i<${{#args[@]}};i++)); do
  [[ "${{args[$i]}}" == "-o" ]] && out="${{args[$((i+1))]}}"
  case "${{args[$i]}}" in *.pdf) [[ -f "${{args[$i]}}" ]] && src="${{args[$i]}}";; esac
done
{halve}"#));

        // ffmpeg ... -i <in> ... <out>   (out is always the last arg)
        stub(&dir, "ffmpeg",
            r#"args=("$@"); inp=""; out="${args[${#args[@]}-1]}"
for ((i=0;i<${#args[@]};i++)); do [[ "${args[$i]}" == "-i" ]] && inp="${args[$((i+1))]}"; done
sz=$(wc -c < "$inp"); head -c $((sz/2 + 1)) "$inp" > "$out""#);

        // vips resize|crop <in> <out> ...   -> copy in to out
        stub(&dir, "vips", r#"cp "$2" "$3""#);

        // vipsthumbnail -s WxH [--smartcrop x] -o <out> <src>
        stub(&dir, "vipsthumbnail",
            r#"out=""; args=("$@"); src="${args[${#args[@]}-1]}"
for ((i=0;i<${#args[@]};i++)); do [[ "${args[$i]}" == "-o" ]] && out="${args[$((i+1))]}"; done
out="${out%%\[*}"; cp "$src" "$out""#);

        // cwebp [opts] <src> -o <out>
        stub(&dir, "cwebp",
            r#"out=""; args=("$@")
for ((i=0;i<${#args[@]};i++)); do [[ "${args[$i]}" == "-o" ]] && out="${args[$((i+1))]}"; done
printf 'WEBP-STUB' > "$out""#);

        // heif-enc [--avif] -q Q -o <out> <src>
        stub(&dir, "heif-enc",
            r#"out=""; args=("$@")
for ((i=0;i<${#args[@]};i++)); do [[ "${args[$i]}" == "-o" ]] && out="${args[$((i+1))]}"; done
printf 'HEIF-STUB' > "$out""#);

        // cjxl ... <src> <out>   (last two args)
        stub(&dir, "cjxl",
            r#"args=("$@"); out="${args[${#args[@]}-1]}"; printf 'JXL-STUB' > "$out""#);

        // exiftool: no-op success
        stub(&dir, "exiftool", "exit 0");

        dir
    });
    xpress_core::tools::set_bin_dir_override(dir.clone());
}

/// Decode the embedded 4x4 PNG to `path`.
pub fn write_png(path: &Path) {
    let bytes = base64_decode(PNG_4X4_BASE64);
    std::fs::write(path, bytes).unwrap();
}

/// Write a dummy file of `size` bytes with the given extension content.
pub fn write_dummy(path: &Path, size: usize) {
    std::fs::write(path, vec![b'x'; size]).unwrap();
}

/// A minimal base64 decoder (avoids adding a dependency to the test harness).
fn base64_decode(s: &str) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for &c in s.as_bytes() {
        if c == b'=' {
            break;
        }
        let v = lookup[c as usize];
        if v == 255 {
            continue;
        }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}
