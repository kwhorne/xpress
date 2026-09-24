//! "Already optimised" markers (extended attributes).

use std::path::{Path, PathBuf};

use xpress_core::audio::AudioFormat;
use xpress_core::compression::CompressionQuality;
use xpress_core::result::OptimiseOptions;
use xpress_core::{optimise_file, pipeline};

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xpress-cache-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Whether this filesystem stores extended attributes (tests are skipped if not).
fn xattrs_supported(dir: &Path) -> bool {
    let probe = dir.join("probe");
    std::fs::write(&probe, b"x").unwrap();
    let name = if cfg!(target_os = "linux") {
        "user.xpress.probe"
    } else {
        "com.xpress.probe"
    };
    xattr::set(&probe, name, b"1").is_ok()
}

fn write_png(path: &Path, seed: u32) {
    let img = image::RgbImage::from_fn(96, 96, |x, y| {
        image::Rgb([(x * 2 + seed) as u8, (y * 2) as u8, (x + y + seed) as u8])
    });
    img.save(path).unwrap();
}

fn opts(factor: i32) -> OptimiseOptions {
    OptimiseOptions {
        compression: CompressionQuality::factor(factor),
        backup: false,
        allow_larger: true,
        ..Default::default()
    }
}

fn run(path: &Path, o: &OptimiseOptions) -> xpress_core::result::OptimisationResult {
    optimise_file(path, o, AudioFormat::SameAsInput, None).unwrap()
}

#[test]
fn second_run_is_skipped_until_the_file_changes() {
    let dir = tmpdir("skip");
    if !xattrs_supported(&dir) {
        return;
    }
    let f = dir.join("shot.png");
    write_png(&f, 0);

    assert!(!run(&f, &opts(30)).cached);
    let after_first = std::fs::read(&f).unwrap();

    let second = run(&f, &opts(30));
    assert!(second.cached, "already optimised with these settings");
    assert_eq!(std::fs::read(&f).unwrap(), after_first, "untouched");

    // Any edit invalidates the marker.
    write_png(&f, 7);
    assert!(!run(&f, &opts(30)).cached);
}

#[test]
fn stronger_settings_run_again_gentler_ones_skip() {
    let dir = tmpdir("settings");
    if !xattrs_supported(&dir) {
        return;
    }
    let f = dir.join("shot.png");
    write_png(&f, 1);
    run(&f, &opts(50));

    assert!(run(&f, &opts(30)).cached, "already compressed harder");
    assert!(!run(&f, &opts(80)).cached, "more compression requested");
    let strip = OptimiseOptions {
        strip_metadata: true,
        ..opts(30)
    };
    assert!(!run(&f, &strip).cached, "metadata stripping requested");
}

#[test]
fn force_ignores_the_marker() {
    let dir = tmpdir("force");
    if !xattrs_supported(&dir) {
        return;
    }
    let f = dir.join("shot.png");
    write_png(&f, 2);
    run(&f, &opts(30));
    let forced = OptimiseOptions {
        use_cache: false,
        ..opts(30)
    };
    assert!(!run(&f, &forced).cached);
}

#[test]
fn cache_hit_still_writes_an_explicit_output() {
    let dir = tmpdir("output");
    if !xattrs_supported(&dir) {
        return;
    }
    let f = dir.join("shot.png");
    write_png(&f, 3);
    run(&f, &opts(30));

    let out = dir.join("copy.png");
    let r = run(
        &f,
        &OptimiseOptions {
            output: Some(out.clone()),
            ..opts(30)
        },
    );
    assert!(r.cached);
    assert_eq!(r.output, out);
    assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(&f).unwrap());
}

#[test]
fn pipeline_on_a_marked_file_still_works() {
    let dir = tmpdir("pipeline");
    if !xattrs_supported(&dir) {
        return;
    }
    let f = dir.join("shot.png");
    write_png(&f, 4);
    run(&f, &opts(30));

    let steps = pipeline::parse("optimise -> convert(to: webp)").unwrap();
    let r = pipeline::run(&f, &steps, &opts(30)).unwrap();
    assert!(r.output.exists());
}
