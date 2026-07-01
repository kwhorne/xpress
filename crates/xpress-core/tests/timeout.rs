//! Isolated test for the external-tool timeout. Lives in its own test binary so
//! its `set_bin_dir_override` / `set_timeout` process state doesn't interfere
//! with the shared stub harness in `integration.rs`.

use std::time::{Duration, Instant};

#[test]
fn tool_timeout_kills_slow_process() {
    let dir = std::env::temp_dir().join(format!("xpress-timeout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();

    // An ffmpeg that sleeps far longer than the timeout (video still shells out).
    let stub = bin.join("ffmpeg");
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

    let vid = dir.join("a.mp4");
    std::fs::write(&vid, vec![b'x'; 1024]).unwrap();

    let start = Instant::now();
    let res = xpress_core::video::optimise(&vid, &xpress_core::result::OptimiseOptions::default());
    let elapsed = start.elapsed();

    assert!(res.is_err(), "slow tool should fail via timeout");
    assert!(
        elapsed < Duration::from_secs(10),
        "timeout should return promptly, took {elapsed:?}"
    );
    let msg = res.unwrap_err().to_string();
    assert!(msg.contains("timed out"), "unexpected error: {msg}");
}
