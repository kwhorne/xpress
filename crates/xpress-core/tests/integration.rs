//! End-to-end tests of the optimisation engine using stub tools.

mod common;

use std::path::PathBuf;

use xpress_core::audio::AudioFormat;
use xpress_core::crop::CropSpec;
use xpress_core::image::ImageFormat;
use xpress_core::result::{OptimiseError, OptimiseOptions};
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
    let dir = tmpdir("jpg");
    let f = dir.join("photo.jpg");
    common::write_image(&f);
    // Re-encoding may or may not shrink a small synthetic image; just require success.
    let r = image::optimise(&f, &opts()).unwrap();
    assert!(r.output.exists());
    assert!(r.new_size > 0);
}

#[test]
fn optimise_gif() {
    let dir = tmpdir("gif");
    let f = dir.join("anim.gif");
    common::write_image(&f);
    let r = image::optimise(&f, &opts()).unwrap();
    assert!(r.output.exists());
    assert!(r.new_size > 0);
}

#[test]
fn size_guard_keeps_original_when_not_smaller() {
    let dir = tmpdir("guard");
    let f = dir.join("photo.jpg");
    common::write_image(&f); // image crate writes JPEG at ~q75
    let before = std::fs::metadata(&f).unwrap().len();
    // Optimising at the normal preset re-encodes at a higher quality (~85), which
    // grows this already-compressed JPEG, so the size guard keeps the original.
    let r = image::optimise(&f, &opts()).unwrap();
    assert!(!r.improved());
    assert_eq!(r.new_size, before);
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
fn pdf_recompresses_embedded_jpeg() {
    use lopdf::{dictionary, Document, Object, Stream};

    let dir = tmpdir("pdfimg");
    let f = dir.join("scan.pdf");

    // A high-quality (q95) noisy JPEG so recompressing at the normal preset (~q85)
    // clearly shrinks it.
    let (w, h) = (256u32, 256u32);
    let mut img = ::image::RgbImage::new(w, h);
    let mut seed = 0x1234_5678u32;
    for px in img.pixels_mut() {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let r = (seed >> 16) as u8;
        let g = (seed >> 8) as u8;
        let b = seed as u8;
        *px = ::image::Rgb([r, g, b]);
    }
    let mut jpeg = Vec::new();
    ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
        .encode_image(&::image::DynamicImage::ImageRgb8(img))
        .unwrap();

    let mut doc = Document::with_version("1.5");
    let img_stream = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w as i64,
            "Height" => h as i64,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        jpeg.clone(),
    );
    let img_id = doc.add_object(Object::Stream(img_stream));
    let content_id = doc.add_object(Stream::new(
        dictionary! {},
        b"q 256 0 0 256 0 0 cm /Im0 Do Q".to_vec(),
    ));
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 256.into(), 256.into()],
        "Contents" => content_id,
        "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => img_id } },
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc.save(&f).unwrap();

    let before = std::fs::metadata(&f).unwrap().len();
    let r = pdf::optimise(&f, &opts(), None).unwrap();
    assert!(
        r.improved(),
        "expected the embedded JPEG to be recompressed smaller"
    );
    assert!(r.new_size < before);
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
    let dir = tmpdir("pdf");
    let f = dir.join("doc.pdf");
    common::write_pdf(&f); // valid minimal PDF (no images to recompress)
                           // Pure-Rust optimise runs lopdf compression; a minimal PDF may already be
                           // optimal, so just require success and a valid output path.
    let r = pdf::optimise(&f, &opts(), None).unwrap();
    assert!(r.new_size > 0);
    assert!(r.output.exists());
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
fn video_mov_is_kept_when_mp4_would_be_larger() {
    common::install_stubs();
    let dir = tmpdir("video-grow");
    let f = dir.join("clip-grow.mov");
    common::write_dummy(&f, 8000);
    let r = video::optimise(&f, &opts()).unwrap();
    assert!(!r.improved());
    assert_eq!(r.output, f, "original kept in place");
    assert!(f.exists(), "original must not be deleted");
    assert!(!dir.join("clip-grow.mp4").exists());
}

#[test]
fn video_crop_is_kept_even_when_larger() {
    common::install_stubs();
    let dir = tmpdir("video-crop-grow");
    let f = dir.join("clip-grow.mp4");
    common::write_dummy(&f, 8000);
    let r = crop::crop_file(&f, &CropSpec::size(640, 0), &opts()).unwrap();
    assert!(r.output.exists());
    assert!(
        r.new_size > r.old_size,
        "an explicit crop is applied regardless"
    );
}

#[test]
fn video_convert_backs_up_the_source_it_replaces() {
    common::install_stubs();
    let dir = tmpdir("video-convert-backup");
    let f = dir.join("clip.mov");
    common::write_dummy(&f, 8000);
    let r = video::convert_codec(&f, video::VideoCodec::Hevc, &opts(), false).unwrap();
    assert_eq!(r.output, dir.join("clip.mp4"));
    assert!(!f.exists(), "converted in place");
    let backup = dir.join(".clip.mov.orig");
    assert_eq!(r.backup.as_deref(), Some(backup.as_path()));
    assert_eq!(std::fs::metadata(&backup).unwrap().len(), 8000);
}

#[test]
fn audio_aac_falls_back_without_audiotoolbox() {
    common::install_stubs(); // the stub ffmpeg, like Linux builds, lacks aac_at
    let dir = tmpdir("audio-aac");
    let f = dir.join("song.wav");
    common::write_dummy(&f, 4000);
    let r = audio::optimise(&f, &opts(), AudioFormat::Aac, Some(128)).unwrap();
    assert_eq!(r.output, dir.join("song.m4a"));
    assert!(r.output.exists());
}

#[test]
fn pipeline_optimise_never_grows_the_file() {
    common::install_stubs();
    let dir = tmpdir("pipeline-guard");
    let f = dir.join("clip-grow.mp4");
    common::write_dummy(&f, 8000);
    let steps = pipeline::parse("optimise").unwrap();
    let r = pipeline::run(&f, &steps, &opts()).unwrap();
    assert!(!r.improved());
    assert_eq!(std::fs::metadata(&f).unwrap().len(), 8000, "original kept");
    assert!(r.backup.is_none());
}

#[test]
fn pipeline_with_content_change_is_kept_even_if_larger() {
    common::install_stubs();
    let dir = tmpdir("pipeline-keep");
    let f = dir.join("clip-grow.mp4");
    common::write_dummy(&f, 8000);
    let steps = pipeline::parse("capFps(fps: 24)").unwrap();
    let r = pipeline::run(&f, &steps, &opts()).unwrap();
    assert!(r.new_size > 8000);
    assert!(r.backup.is_some(), "original backed up before replacing");
}

fn ffmpeg_log(input: &std::path::Path) -> String {
    let mut log = input.as_os_str().to_owned();
    log.push(".ffmpeg-log");
    std::fs::read_to_string(log).unwrap_or_default()
}

#[test]
fn hdr_video_is_tone_mapped_for_h264() {
    common::install_stubs();
    let dir = tmpdir("video-hdr");
    let f = dir.join("iphone-hdr.mov");
    common::write_dummy(&f, 8000);
    video::optimise(&f, &opts()).unwrap();
    let log = ffmpeg_log(&f);
    let encode = log
        .lines()
        .find(|l| l.contains("-vcodec"))
        .expect("an encode ran");
    assert!(encode.contains("tonemap=tonemap=mobius"), "{encode}");
    assert!(encode.contains("-pix_fmt yuv420p"));
}

#[test]
fn sdr_video_is_not_tone_mapped() {
    common::install_stubs();
    let dir = tmpdir("video-sdr");
    let f = dir.join("clip.mov");
    common::write_dummy(&f, 8000);
    video::optimise(&f, &opts()).unwrap();
    assert!(!ffmpeg_log(&f).contains("tonemap"));
}

#[test]
fn hdr_crop_chains_the_crop_before_tone_mapping() {
    common::install_stubs();
    let dir = tmpdir("video-hdr-crop");
    let f = dir.join("hdr.mp4");
    common::write_dummy(&f, 8000);
    crop::crop_file(&f, &CropSpec::size(640, 0), &opts()).unwrap();
    let log = ffmpeg_log(&f);
    let encode = log.lines().find(|l| l.contains("-vcodec")).unwrap();
    assert!(encode.contains("scale=640:-2,zscale=t=linear"), "{encode}");
}

#[test]
fn hevc_conversion_keeps_hdr() {
    common::install_stubs();
    let dir = tmpdir("video-hdr-hevc");
    let f = dir.join("hdr.mov");
    common::write_dummy(&f, 8000);
    let o = OptimiseOptions {
        output: Some(dir.join("out.mp4")),
        ..opts()
    };
    video::convert_codec(&f, video::VideoCodec::Hevc, &o, false).unwrap();
    assert!(
        !ffmpeg_log(&f).contains("tonemap"),
        "10-bit HEVC can carry HDR"
    );
}

/// The `-b:v` of every second-pass encode in the stub's log, in order.
fn second_pass_bitrates(log: &str) -> Vec<u32> {
    log.lines()
        .filter(|l| l.contains("-pass 2"))
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            it.find(|a| *a == "-b:v")?;
            it.next()?.trim_end_matches('k').parse().ok()
        })
        .collect()
}

#[test]
fn video_budget_encodes_two_pass_at_the_computed_bitrate() {
    common::install_stubs();
    let dir = tmpdir("video-budget");
    let f = dir.join("clip.mov");
    common::write_dummy(&f, 8_000_000); // stub banner: 10 s, 1080p30, AAC
    let r = xpress_core::budget::optimise_to_budget(&f, 5_000_000, &opts()).unwrap();

    assert_eq!(r.output, dir.join("clip.mp4"));
    assert!(r.new_size <= 5_000_000);
    let log = ffmpeg_log(&f);
    assert!(log.contains("-pass 1") && log.contains("-pass 2"), "{log}");
    // 5 MB / 10 s = 4000 kbit/s, -4% = 3840, minus 128 audio.
    assert_eq!(second_pass_bitrates(&log), vec![3712]);
    assert!(log.contains("-c:a aac -b:a 128k"));
    assert!(!log.contains("scale="), "3.7 Mbit/s is plenty for 1080p30");
}

#[test]
fn video_budget_downscales_when_bits_are_thin() {
    common::install_stubs();
    let dir = tmpdir("video-budget-thin");
    let f = dir.join("clip.mp4");
    common::write_dummy(&f, 8_000_000);
    xpress_core::budget::optimise_to_budget(&f, 1_000_000, &opts()).unwrap();
    let log = ffmpeg_log(&f);
    let pass2 = log.lines().find(|l| l.contains("-pass 2")).unwrap();
    assert!(pass2.contains("scale="), "{pass2}");
}

#[test]
fn video_budget_corrects_an_overshoot() {
    common::install_stubs();
    let dir = tmpdir("video-budget-over");
    let f = dir.join("clip.mp4");
    // The stub always outputs half the input (4 MB) whatever the settings, so
    // a 3.5 MB budget overshoots slightly every time: xpress keeps lowering
    // the bitrate (a small overshoot doesn't call for a smaller frame).
    common::write_dummy(&f, 8_000_000);
    let r = xpress_core::budget::optimise_to_budget(&f, 3_500_000, &opts()).unwrap();
    let log = ffmpeg_log(&f);
    let rates = second_pass_bitrates(&log);
    assert_eq!(rates.len(), 4, "three corrections: {rates:?}");
    assert!(rates.windows(2).all(|w| w[1] < w[0]), "{rates:?}");
    assert!(r.new_size > 3_500_000, "still reports the smallest it got");
}

#[test]
fn video_budget_shrinks_the_frame_on_a_big_overshoot() {
    common::install_stubs();
    let dir = tmpdir("video-budget-way-over");
    let f = dir.join("clip.mp4");
    // Output 4 MB vs a 2.5 MB budget (1.6x): the rate can't be the fix.
    common::write_dummy(&f, 8_000_000);
    xpress_core::budget::optimise_to_budget(&f, 2_500_000, &opts()).unwrap();
    let log = ffmpeg_log(&f);
    let heights: Vec<u32> = log
        .lines()
        .filter(|l| l.contains("-pass 2"))
        .filter_map(|l| {
            let s = l.split("scale=").nth(1)?;
            s.split([':', ',']).nth(1)?.parse().ok()
        })
        .collect();
    assert!(heights.len() >= 2, "{log}");
    assert!(heights.windows(2).all(|w| w[1] < w[0]), "{heights:?}");
}

#[test]
fn audio_budget_targets_a_bitrate() {
    common::install_stubs();
    let dir = tmpdir("audio-budget");
    let f = dir.join("song.mp3");
    common::write_dummy(&f, 400_000); // stub banner: 10 s
    xpress_core::budget::optimise_to_budget(&f, 250_000, &opts()).unwrap();
    let log = ffmpeg_log(&f);
    // 250 kB / 10 s = 200 kbit/s -3% = 194 -> LAME VBR quality for <=256k.
    assert!(log.contains("libmp3lame"), "{log}");
}

#[test]
fn budget_already_met_just_optimises() {
    common::install_stubs();
    let dir = tmpdir("budget-met");
    let f = dir.join("clip.mp4");
    common::write_dummy(&f, 8000);
    xpress_core::budget::optimise_to_budget(&f, 1_000_000, &opts()).unwrap();
    assert!(
        !ffmpeg_log(&f).contains("-pass"),
        "no bitrate targeting needed"
    );
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
    let dir = tmpdir("budget");
    let f = dir.join("big.png");
    common::write_image(&f);
    let before = std::fs::metadata(&f).unwrap().len();
    let r = xpress_core::budget::optimise_to_budget(&f, before / 2, &opts()).unwrap();
    assert!(r.new_size < r.old_size, "budget should shrink the file");
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
fn errors_on_missing_file() {
    common::install_stubs();
    let r = image::optimise(std::path::Path::new("/no/such/xpress-file.png"), &opts());
    assert!(matches!(r, Err(OptimiseError::NotFound(_))));
}

#[test]
fn errors_on_unsupported_type() {
    common::install_stubs();
    let dir = tmpdir("unsup");
    let f = dir.join("notes.txt");
    common::write_dummy(&f, 32);
    let r = xpress_core::optimise_file(&f, &opts(), AudioFormat::SameAsInput, None);
    assert!(matches!(r, Err(OptimiseError::Unsupported(_))));
}

#[test]
fn crop_errors_on_unreadable_dimensions() {
    common::install_stubs();
    let dir = tmpdir("baddim");
    let f = dir.join("x.png"); // not a real PNG -> imagesize can't read dims
    common::write_dummy(&f, 20);
    let r = crop::crop_file(&f, &CropSpec::parse("2x2").unwrap(), &opts());
    assert!(r.is_err(), "crop should fail when dimensions can't be read");
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
