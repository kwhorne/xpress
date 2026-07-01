//! Isolated test for the external-tool timeout. Lives in its own test binary so
//! its `set_bin_dir_override` / `set_timeout` process state doesn't interfere
//! with the shared stub harness in `integration.rs`.

use std::time::{Duration, Instant};

const PNG_4X4_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAEUlEQVR4nGNkYPhfz0BHwDhqGAMABzMCpQ4P0XwAAAAASUVORK5CYII=";

fn base64_decode(s: &str) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }
    let (mut buf, mut bits, mut out) = (0u32, 0, Vec::new());
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

#[test]
fn tool_timeout_kills_slow_process() {
    let dir = std::env::temp_dir().join(format!("xpress-timeout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();

    // A pngquant that sleeps far longer than the timeout.
    let stub = bin.join("pngquant");
    std::fs::write(&stub, "#!/bin/bash\nsleep 30\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&stub).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&stub, p).unwrap();
    }

    xpress_core::tools::set_bin_dir_override(bin);
    xpress_core::tools::set_timeout(Some(Duration::from_secs(1)));

    let img = dir.join("a.png");
    std::fs::write(&img, base64_decode(PNG_4X4_BASE64)).unwrap();

    let start = Instant::now();
    let res = xpress_core::image::optimise(&img, &xpress_core::result::OptimiseOptions::default());
    let elapsed = start.elapsed();

    assert!(res.is_err(), "slow tool should fail via timeout");
    assert!(
        elapsed < Duration::from_secs(10),
        "timeout should return promptly, took {elapsed:?}"
    );
    let msg = res.unwrap_err().to_string();
    assert!(msg.contains("timed out"), "unexpected error: {msg}");
}
