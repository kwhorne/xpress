//! End-to-end tests of the optimisation engine using stub tools.

mod common;

use std::path::PathBuf;

use xpress_core::audio::AudioFormat;
use xpress_core::crop::CropSpec;
use xpress_core::image::ImageFormat;
use xpress_core::result::OptimiseOptions;
use xpress_core::{audio, crop, image, pdf, pipeline, scale, video};

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xpress-it-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn opts() -> OptimiseOptions {
    OptimiseOptions {
        backup: true,
        ..Default::default()
    }
}

#[test]
fn optimise_png_in_place_with_backup() {
    common::install_stubs();
    let dir = tmpdir("png");
    let f = dir.join("shot.png");
    common::write_png(&f);
    let before = std::fs::metadata(&f).unwrap().len();

    let r = image::optimise(&f, &opts()).unwrap();

    assert!(r.improved(), "expected a smaller file");
    assert!(r.new_size < before);
    assert_eq!(r.output, f);
    let backup = dir.join(".shot.png.orig");
    assert!(backup.exists(), "original should be backed up");
    assert_eq!(std::fs::metadata(&backup).unwrap().len(), before);
}

#[test]
fn optimise_jpeg() {
    common::install_stubs();
    let dir = tmpdir("jpg");
    let f = dir.join("photo.jpg");
    common::write_dummy(&f, 2000);
    let r = image::optimise(&f, &opts()).unwrap();
    assert!(r.improved());
}

#[test]
fn optimise_gif() {
    common::install_stubs();
    let dir = tmpdir("gif");
    let f = dir.join("anim.gif");
    common::write_dummy(&f, 2000);
    let r = image::optimise(&f, &opts()).unwrap();
    assert!(r.improved());
}

#[test]
fn size_guard_keeps_original_when_not_smaller() {
    common::install_stubs();
    let dir = tmpdir("guard");
    let f = dir.join("tiny.png");
    common::write_dummy(&f, 2); // halving 2 bytes -> ~2 bytes, not smaller
    let r = image::optimise(&f, &opts()).unwrap();
    assert!(!r.improved());
    assert_eq!(r.new_size, r.old_size);
    assert!(r.backup.is_none(), "no backup when nothing changed");
}

#[test]
fn optimise_to_explicit_output_makes_no_backup() {
    common::install_stubs();
    let dir = tmpdir("out");
    let f = dir.join("a.png");
    common::write_png(&f);
    let out = dir.join("b.png");
    let o = OptimiseOptions {
        output: Some(out.clone()),
        ..opts()
    };
    let r = image::optimise(&f, &o).unwrap();
    assert_eq!(r.output, out);
    assert!(out.exists());
    assert!(r.backup.is_none());
    assert!(f.exists(), "source untouched when writing elsewhere");
}

#[test]
fn convert_png_to_webp() {
    common::install_stubs();
    let dir = tmpdir("conv");
    let f = dir.join("pic.png");
    common::write_png(&f);
    let r = image::convert(&f, ImageFormat::Webp, &opts()).unwrap();
    assert_eq!(r.output.extension().unwrap(), "webp");
    assert!(r.output.exists());
    assert!(f.exists(), "original kept after conversion");
}

#[test]
fn pdf_crop_and_uncrop() {
    common::install_stubs();
    let dir = tmpdir("pdfcrop");
    let f = dir.join("doc.pdf");
    common::write_pdf(&f);

    let cropped = dir.join("cropped.pdf");
    pdf::crop(&f, &cropped, (16.0, 9.0)).unwrap();
    let bytes = std::fs::read(&cropped).unwrap();
    assert!(
        bytes.windows(7).any(|w| w == b"CropBox"),
        "crop should add a CropBox"
    );

    let uncropped = dir.join("uncropped.pdf");
    pdf::uncrop(&cropped, &uncropped).unwrap();
    let bytes2 = std::fs::read(&uncropped).unwrap();
    assert!(
        !bytes2.windows(7).any(|w| w == b"CropBox"),
        "uncrop should remove the CropBox"
    );
}

#[test]
fn pdf_optimise() {
    common::install_stubs();
    let dir = tmpdir("pdf");
    let f = dir.join("doc.pdf");
    common::write_dummy(&f, 4000);
    let r = pdf::optimise(&f, &opts(), Some(144)).unwrap();
    assert!(r.improved());
}

#[test]
fn audio_convert_to_mp3() {
    common::install_stubs();
    let dir = tmpdir("audio");
    let f = dir.join("song.wav");
    common::write_dummy(&f, 4000);
    let r = audio::optimise(&f, &opts(), AudioFormat::Mp3, Some(128)).unwrap();
    assert_eq!(r.output.extension().unwrap(), "mp3");
    assert!(r.output.exists());
}

#[test]
fn video_optimise_normalises_to_mp4() {
    common::install_stubs();
    let dir = tmpdir("video");
    let f = dir.join("clip.mov");
    common::write_dummy(&f, 8000);
    let r = video::optimise(&f, &opts()).unwrap();
    assert_eq!(r.output.extension().unwrap(), "mp4");
    assert!(r.output.exists());
}

#[test]
fn downscale_image_by_factor() {
    common::install_stubs();
    let dir = tmpdir("scale");
    let f = dir.join("big.png");
    common::write_png(&f);
    let r = scale::downscale_file(&f, 0.5, &opts()).unwrap();
    assert!(r.new_size < r.old_size);
}

#[test]
fn crop_image_to_size() {
    common::install_stubs();
    let dir = tmpdir("crop");
    let f = dir.join("pic.png");
    common::write_png(&f);
    let spec = CropSpec::parse("2x2").unwrap();
    let r = crop::crop_file(&f, &spec, &opts()).unwrap();
    assert_eq!(r.output, f);
    assert!(r.new_size > 0);
}

#[test]
fn pipeline_crop_then_convert() {
    common::install_stubs();
    let dir = tmpdir("pipe");
    let f = dir.join("s.png");
    common::write_png(&f);
    let steps = pipeline::parse("crop(width: 2) -> convert(to: webp)").unwrap();
    let r = pipeline::run(&f, &steps, &opts()).unwrap();
    assert_eq!(r.output.extension().unwrap(), "webp");
    assert!(r.output.exists());
    assert!(f.exists(), "original kept (type changed)");
}

#[test]
fn video_to_gif() {
    common::install_stubs();
    let dir = tmpdir("vgif");
    let f = dir.join("clip.mov");
    common::write_dummy(&f, 8000);
    let r = video::to_gif(&f, &opts(), 15, None).unwrap();
    assert_eq!(r.output.extension().unwrap(), "gif");
    assert!(r.output.exists());
}

#[test]
fn video_convert_hevc() {
    common::install_stubs();
    let dir = tmpdir("hevc");
    let f = dir.join("clip.mov");
    common::write_dummy(&f, 8000);
    let r = video::convert_codec(&f, video::VideoCodec::Hevc, &opts(), false).unwrap();
    assert_eq!(r.output.extension().unwrap(), "mp4");
    assert!(r.output.exists());
}

#[test]
fn video_convert_vp9_webm() {
    common::install_stubs();
    let dir = tmpdir("vp9");
    let f = dir.join("clip.mov");
    common::write_dummy(&f, 8000);
    let r = video::convert_codec(&f, video::VideoCodec::Vp9, &opts(), false).unwrap();
    assert_eq!(r.output.extension().unwrap(), "webm");
}

#[test]
fn alpha_detection() {
    common::install_stubs();
    let dir = tmpdir("alpha");
    let jpg = dir.join("x.jpg");
    common::write_dummy(&jpg, 100);
    assert!(!image::has_alpha(&jpg), "non-png reports no alpha");
}

#[test]
fn target_size_budget() {
    common::install_stubs();
    let dir = tmpdir("budget");
    let f = dir.join("big.png");
    common::write_dummy(&f, 10_000);
    let r = xpress_core::budget::optimise_to_budget(&f, 1_000, &opts()).unwrap();
    assert!(r.new_size < r.old_size);
}

#[test]
fn adaptive_picks_smallest() {
    common::install_stubs();
    let dir = tmpdir("adaptive");
    let f = dir.join("pic.png");
    common::write_png(&f);
    let r = image::optimise_adaptive(&f, &opts()).unwrap();
    assert!(r.new_size > 0);
    assert!(r.output.exists());
}

#[test]
fn template_expands_output() {
    let mut c = 1;
    let out =
        xpress_core::template::expand("%f-small.%e", std::path::Path::new("/x/photo.png"), &mut c);
    assert_eq!(out, std::path::PathBuf::from("photo-small.png"));
}

#[test]
fn pipeline_run_script_passthrough() {
    common::install_stubs();
    let dir = tmpdir("script");
    let f = dir.join("s.png");
    common::write_png(&f);
    let marker = dir.join("ran.txt");
    let steps =
        pipeline::parse(&format!("runScript(code: \"touch {}\")", marker.display())).unwrap();
    let r = pipeline::run(&f, &steps, &opts()).unwrap();
    assert!(r.output.exists());
    assert!(marker.exists(), "script should have run");
}

#[test]
fn pipeline_normalize_audio() {
    common::install_stubs();
    let dir = tmpdir("norm");
    let f = dir.join("a.mp3");
    common::write_dummy(&f, 4000);
    let steps = pipeline::parse("normalize(lufs: -16)").unwrap();
    let r = pipeline::run(&f, &steps, &opts()).unwrap();
    assert!(r.output.exists());
}

#[test]
fn backup_then_restore_roundtrip() {
    common::install_stubs();
    let dir = tmpdir("restore");
    let f = dir.join("a.png");
    common::write_png(&f);
    let original = std::fs::metadata(&f).unwrap().len();

    image::optimise(&f, &opts()).unwrap();
    assert!(std::fs::metadata(&f).unwrap().len() < original);

    let backups = xpress_core::result::find_backups(std::slice::from_ref(&dir), false);
    assert_eq!(backups.len(), 1);
    let (backup, orig_path) = &backups[0];
    assert_eq!(orig_path, &f);
    std::fs::rename(backup, orig_path).unwrap();
    assert_eq!(std::fs::metadata(&f).unwrap().len(), original);
}

#[test]
fn collect_files_filters_by_kind() {
    common::install_stubs();
    let dir = tmpdir("collect");
    common::write_png(&dir.join("a.png"));
    common::write_dummy(&dir.join("b.mp3"), 100);
    common::write_dummy(&dir.join("c.txt"), 100);
    let imgs = xpress_core::collect_files(
        std::slice::from_ref(&dir),
        false,
        &[xpress_core::filetype::MediaKind::Image],
    );
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].file_name().unwrap(), "a.png");
}
