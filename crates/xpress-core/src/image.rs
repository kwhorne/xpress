//! Image optimisation: JPEG (jpegoptim), PNG (pngquant), GIF (gifsicle).

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::compression::CompressionQuality;
use crate::filetype::{extension_lower, MediaKind};
use crate::result::{
    backup_file, copy_dates, file_name_lossy, file_size, file_stem_lossy, OptimisationResult,
    OptimiseError, OptimiseOptions,
};
use crate::tools::{self, Tool};

/// Optimise an image in place (or to `options.output`).
pub fn optimise(
    path: &Path,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let ext =
        extension_lower(path).ok_or_else(|| OptimiseError::Unsupported(path.to_path_buf()))?;
    let old_size = file_size(path);
    let cq = options.compression;

    let tmp = TempDir::new()?;
    let temp_out = tmp.path().join(file_name_lossy(path));

    match ext.as_str() {
        "jpg" | "jpeg" => optimise_jpeg(path, &temp_out, cq)?,
        "png" => optimise_png(path, &temp_out, cq)?,
        "gif" => optimise_gif(path, &temp_out, cq)?,
        _ => return Err(OptimiseError::Unsupported(path.to_path_buf())),
    }

    finalise(path, &temp_out, old_size, cq, options)
}

/// jpegoptim --keep-all --force --max <q> --auto-mode(arm) --overwrite --dest <dir> <file>
fn optimise_jpeg(src: &Path, out: &Path, cq: CompressionQuality) -> Result<(), OptimiseError> {
    std::fs::copy(src, out)?;
    let dest_dir = out.parent().unwrap().to_path_buf();
    let mut args: Vec<String> = vec![
        "--keep-all".into(),
        "--force".into(),
        "--max".into(),
        cq.jpeg_max_quality().to_string(),
    ];
    if tools::is_arm64() {
        args.push("--auto-mode".into());
    }
    args.push("--overwrite".into());
    args.push("--dest".into());
    args.push(dest_dir.display().to_string());
    args.push(out.display().to_string());

    let res = tools::run_with_retries(Tool::Jpegoptim, &args, 2);
    if res.is_err() {
        // Fallback: jpegoptim-old with the secondary quality ceiling.
        let args2: Vec<String> = vec![
            "--keep-all".into(),
            "--force".into(),
            "--max".into(),
            cq.jpeg_secondary_max_quality().to_string(),
            "--auto-mode".into(),
            "--overwrite".into(),
            "--dest".into(),
            dest_dir.display().to_string(),
            out.display().to_string(),
        ];
        tools::run_with_retries(Tool::JpegoptimOld, &args2, 2)?;
    }
    Ok(())
}

/// pngquant --force --speed <s> --quality <0-max> --output <out> <src>
fn optimise_png(src: &Path, out: &Path, cq: CompressionQuality) -> Result<(), OptimiseError> {
    let args: Vec<String> = vec![
        "--force".into(),
        "--speed".into(),
        cq.pngquant_speed().to_string(),
        "--quality".into(),
        cq.pngquant_quality(),
        "--output".into(),
        out.display().to_string(),
        src.display().to_string(),
    ];
    tools::run_with_retries(Tool::Pngquant, &args, 2)?;
    Ok(())
}

/// gifsicle <args> --threads=N --output <out> <src>
fn optimise_gif(src: &Path, out: &Path, cq: CompressionQuality) -> Result<(), OptimiseError> {
    let mut args = cq.gifsicle_args();
    args.push(format!("--threads={}", tools::cpu_count()));
    args.push("--output".into());
    args.push(out.display().to_string());
    args.push(src.display().to_string());
    tools::run_with_retries(Tool::Gifsicle, &args, 3)?;
    Ok(())
}

/// Adaptively optimise an image: try the in-format optimisation and a JPEG and a
/// PNG candidate, then keep whichever is smallest. Mirrors the idea of routing
/// detail-heavy images to JPEG and flat ones to PNG.
pub fn optimise_adaptive(
    path: &Path,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let old_size = file_size(path);
    let tmp = TempDir::new()?;

    // Candidate 1: optimise in the original format (unique temp name).
    let src_ext = extension_lower(path).unwrap_or_else(|| "png".into());
    let c_same = tmp.path().join(format!("c0_same.{src_ext}"));
    let same_opts = OptimiseOptions {
        output: Some(c_same.clone()),
        backup: false,
        allow_larger: true,
        ..options.clone()
    };
    let mut candidates: Vec<(PathBuf, ImageFormat)> = Vec::new();
    if optimise(path, &same_opts).is_ok() {
        // Track by extension; format only matters for the eventual placement ext.
        let f = ImageFormat::from_str(&src_ext).unwrap_or(ImageFormat::Png);
        candidates.push((c_same, f));
    }

    // Candidate 2 + 3: JPEG and PNG conversions (unique temp names).
    for (i, fmt) in [ImageFormat::Jpeg, ImageFormat::Png]
        .into_iter()
        .enumerate()
    {
        let c = tmp.path().join(format!("c{}.{}", i + 1, fmt.extension()));
        let opts = OptimiseOptions {
            output: Some(c.clone()),
            backup: false,
            allow_larger: true,
            ..options.clone()
        };
        if convert(path, fmt, &opts).is_ok() {
            candidates.push((c, fmt));
        }
    }

    // Pick the smallest candidate.
    let (best_path, best_fmt) = candidates
        .into_iter()
        .filter(|(p, _)| file_size(p) > 0)
        .min_by_key(|(p, _)| file_size(p))
        .ok_or_else(|| OptimiseError::Other("adaptive: no candidate produced".into()))?;
    let new_size = file_size(&best_path);

    // Honour the size guard against the original.
    if !options.allow_larger && new_size >= old_size {
        return Ok(OptimisationResult {
            kind: MediaKind::Image,
            source: path.to_path_buf(),
            output: path.to_path_buf(),
            backup: None,
            old_size,
            new_size: old_size,
            aggressive: options.compression.image_is_aggressive(),
        });
    }

    let same_ext = extension_lower(path).as_deref() == Some(best_fmt.extension());
    let dest = options.output.clone().unwrap_or_else(|| {
        if same_ext {
            path.to_path_buf()
        } else {
            path.with_extension(best_fmt.extension())
        }
    });
    let backup = if options.backup && options.output.is_none() && same_ext {
        Some(backup_file(path)?)
    } else {
        None
    };
    std::fs::copy(&best_path, &dest)?;
    if options.preserve_dates {
        copy_dates(path, &dest);
    }

    Ok(OptimisationResult {
        kind: MediaKind::Image,
        source: path.to_path_buf(),
        output: dest,
        backup,
        old_size,
        new_size,
        aggressive: options.compression.image_is_aggressive(),
    })
}

/// Target formats for image conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Webp,
    Avif,
    Heic,
    Jxl,
    Png,
    Jpeg,
}

impl ImageFormat {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<ImageFormat> {
        match s.to_ascii_lowercase().as_str() {
            "webp" => Some(ImageFormat::Webp),
            "avif" => Some(ImageFormat::Avif),
            "heic" | "heif" => Some(ImageFormat::Heic),
            "jxl" => Some(ImageFormat::Jxl),
            "png" => Some(ImageFormat::Png),
            "jpeg" | "jpg" => Some(ImageFormat::Jpeg),
            _ => None,
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            ImageFormat::Webp => "webp",
            ImageFormat::Avif => "avif",
            ImageFormat::Heic => "heic",
            ImageFormat::Jxl => "jxl",
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }
}

/// Convert an image to another format. The output is optimised by the target
/// encoder's own quality setting (derived from the compression value).
pub fn convert(
    path: &Path,
    format: ImageFormat,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let old_size = file_size(path);
    let cq = options.compression;
    let q = cq.conversion_quality();

    let tmp = TempDir::new()?;
    let temp_out = tmp
        .path()
        .join(format!("{}.{}", file_stem_lossy(path), format.extension()));
    let src = path.display().to_string();
    let out = temp_out.display().to_string();

    match format {
        ImageFormat::Webp => {
            // cwebp -mt -q <q> -sharp_yuv -metadata all <src> -o <out>
            tools::run_with_retries(
                Tool::Cwebp,
                [
                    "-mt",
                    "-q",
                    &q.to_string(),
                    "-sharp_yuv",
                    "-metadata",
                    "all",
                    &src,
                    "-o",
                    &out,
                ],
                2,
            )?;
        }
        ImageFormat::Avif => {
            // heif-enc --avif -q <q> -o <out> <src>
            tools::run_with_retries(
                Tool::HeifEnc,
                ["--avif", "-q", &q.to_string(), "-o", &out, &src],
                2,
            )?;
        }
        ImageFormat::Heic => {
            // heif-enc -q <q> -o <out> <src>
            tools::run_with_retries(Tool::HeifEnc, ["-q", &q.to_string(), "-o", &out, &src], 2)?;
        }
        ImageFormat::Jxl => {
            // cjxl -q <quality> -e <effort> <src> <out>
            tools::run_with_retries(
                Tool::Cjxl,
                [
                    "-q",
                    &cq.jxl_quality().to_string(),
                    "-e",
                    &cq.jxl_effort().to_string(),
                    &src,
                    &out,
                ],
                2,
            )?;
        }
        ImageFormat::Png | ImageFormat::Jpeg => {
            // Re-encode with ffmpeg into a distinct raw file, then optimise that
            // into `temp_out` (never optimise a file in place onto itself).
            let raw = tmp.path().join(format!(
                "raw_{}",
                temp_out.file_name().unwrap().to_string_lossy()
            ));
            tools::run(Tool::Ffmpeg, ["-y", "-i", &src, &raw.display().to_string()])?;
            match format {
                ImageFormat::Png => optimise_png(&raw, &temp_out, cq)?,
                ImageFormat::Jpeg => optimise_jpeg(&raw, &temp_out, cq)?,
                _ => unreachable!(),
            }
        }
    }

    let new_size = file_size(&temp_out);
    let dest = options
        .output
        .clone()
        .unwrap_or_else(|| path.with_extension(format.extension()));

    if !options.strip_metadata {
        tools::copy_exif(path, &temp_out, false, &[]);
    }
    std::fs::copy(&temp_out, &dest)?;
    if options.preserve_dates {
        copy_dates(path, &dest);
    }

    Ok(OptimisationResult {
        kind: MediaKind::Image,
        source: path.to_path_buf(),
        output: dest,
        backup: None,
        old_size,
        new_size,
        aggressive: cq.image_is_aggressive(),
    })
}

/// Move the temp output into place, honouring backup / dates / size guard.
fn finalise(
    src: &Path,
    temp_out: &Path,
    old_size: u64,
    cq: CompressionQuality,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    let new_size = file_size(temp_out);

    // Refuse to write a larger file unless explicitly allowed.
    if !options.allow_larger && new_size >= old_size {
        return Ok(OptimisationResult {
            kind: MediaKind::Image,
            source: src.to_path_buf(),
            output: src.to_path_buf(),
            backup: None,
            old_size,
            new_size: old_size,
            aggressive: cq.image_is_aggressive(),
        });
    }

    let backup = if options.backup && options.output.is_none() {
        Some(backup_file(src)?)
    } else {
        None
    };

    if options.strip_metadata {
        tools::copy_exif(src, temp_out, true, &[]);
    } else {
        tools::copy_exif(src, temp_out, false, &[]);
    }

    let dest = options.output.clone().unwrap_or_else(|| src.to_path_buf());
    std::fs::copy(temp_out, &dest)?;
    if options.preserve_dates {
        copy_dates(src, &dest);
    }

    Ok(OptimisationResult {
        kind: MediaKind::Image,
        source: src.to_path_buf(),
        output: dest,
        backup,
        old_size,
        new_size,
        aggressive: cq.image_is_aggressive(),
    })
}
