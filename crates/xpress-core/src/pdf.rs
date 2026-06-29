//! PDF optimisation via ghostscript (pdfwrite device).

use std::path::Path;

use tempfile::TempDir;

use crate::filetype::MediaKind;
use crate::result::{
    backup_file, copy_dates, file_size, OptimisationResult, OptimiseError, OptimiseOptions,
};
use crate::tools::{self, Tool};

pub const PDF_DPI_NO_DOWNSAMPLE: i32 = 300;
pub const PDF_DPI_MIN: i32 = 48;
pub const PDF_DPI_MAX: i32 = 300;

fn gs_base_args() -> Vec<&'static str> {
    vec![
        "-dALLOWPSTRANSPARENCY",
        "-dAutoRotatePages=/None",
        "-dBATCH",
        "-dCannotEmbedFontPolicy=/Warning",
        "-dColorConversionStrategy=/sRGB",
        "-dCompatibilityLevel=1.6",
        "-dCompressFonts=true",
        "-dCompressPages=true",
        "-dCompressStreams=true",
        "-dConvertCMYKImagesToRGB=true",
        "-dConvertImagesToIndexed=false",
        "-dCreateJobTicket=false",
        "-dDetectDuplicateImages=true",
        "-dDoThumbnails=false",
        "-dEmbedAllFonts=true",
        "-dEncodeColorImages=true",
        "-dEncodeGrayImages=true",
        "-dEncodeMonoImages=true",
        "-dFastWebView=false",
        "-dGrayDetection=true",
        "-dHaveTransparency=true",
        "-dLZWEncodePages=true",
        "-dMaxBitmap=0",
        "-dMonoImageFilter=/CCITTFaxEncode",
        "-dNOPAUSE",
        "-dNOPROMPT",
        "-dOptimize=true",
        "-dParseDSCComments=false",
        "-dParseDSCCommentsForDocInfo=false",
        "-dPDFNOCIDFALLBACK",
        "-dPDFSETTINGS=/screen",
        "-dPreserveAnnots=true",
        "-dPreserveCopyPage=false",
        "-dPreserveDeviceN=true",
        "-dPreserveEPSInfo=false",
        "-dPreserveHalftoneInfo=false",
        "-dPreserveOPIComments=false",
        "-dPreserveOverprintSettings=true",
        "-dPreserveSeparation=true",
        "-dPrinted=false",
        "-dProcessColorModel=/DeviceRGB",
        "-dSAFER",
        "-dSubsetFonts=true",
        "-dTransferFunctionInfo=/Apply",
        "-dUCRandBGInfo=/Remove",
    ]
}

fn gs_resolution_args(dpi: i32) -> Vec<String> {
    vec![
        "-dColorImageDownsampleThreshold=1.0".into(),
        "-dColorImageDownsampleType=/Bicubic".into(),
        format!("-dColorImageResolution={dpi}"),
        "-dGrayImageDownsampleThreshold=1.0".into(),
        "-dGrayImageDownsampleType=/Bicubic".into(),
        format!("-dGrayImageResolution={dpi}"),
        "-dMonoImageDownsampleThreshold=1.0".into(),
        "-dMonoImageDownsampleType=/Bicubic".into(),
        // Mono (1-bit) images compress poorly below 300 dpi; clamp.
        format!("-dMonoImageResolution={}", dpi.max(300)),
    ]
}

fn gs_lossy_args(downsample: bool) -> Vec<String> {
    vec![
        "-dAutoFilterColorImages=false".into(),
        "-dAutoFilterGrayImages=false".into(),
        "-dAutoFilterMonoImages=true".into(),
        "-dColorImageFilter=/DCTEncode".into(),
        format!("-dDownsampleColorImages={downsample}"),
        format!("-dDownsampleGrayImages={downsample}"),
        format!("-dDownsampleMonoImages={downsample}"),
        "-dGrayImageFilter=/DCTEncode".into(),
        "-dPassThroughJPEGImages=false".into(),
        "-dPassThroughJPXImages=false".into(),
        "-dShowAcroForm=false".into(),
    ]
}

fn gs_lossless_args(downsample: bool) -> Vec<String> {
    let pass = (!downsample).to_string();
    vec![
        "-dAutoFilterColorImages=false".into(),
        "-dAutoFilterGrayImages=false".into(),
        "-dAutoFilterMonoImages=false".into(),
        "-dColorImageFilter=/DCTEncode".into(),
        format!("-dDownsampleColorImages={downsample}"),
        format!("-dDownsampleGrayImages={downsample}"),
        format!("-dDownsampleMonoImages={downsample}"),
        "-dGrayImageFilter=/DCTEncode".into(),
        format!("-dPassThroughJPEGImages={pass}"),
        format!("-dPassThroughJPXImages={pass}"),
        "-dShowAcroForm=true".into(),
    ]
}

const GS_PRE_DISTILLER: &str = "<< /ColorImageDict << /QFactor 0.68 /Blend 1 /HSamples [2 1 1 2] /VSamples [2 1 1 2] >> >> setdistillerparams << /ColorACSImageDict << /QFactor 0.68 /Blend 1 /HSamples [2 1 1 2] /VSamples [2 1 1 2] >> >> setdistillerparams << /GrayImageDict << /QFactor 0.68 /Blend 1 /HSamples [2 1 1 2] /VSamples [2 1 1 2] >> >> setdistillerparams << /GrayACSImageDict << /QFactor 0.68 /Blend 1 /HSamples [2 1 1 2] /VSamples [2 1 1 2] >> >> setdistillerparams << /AlwaysEmbed [ ] >> setdistillerparams << /NeverEmbed [/Courier /Courier-Bold /Courier-Oblique /Courier-BoldOblique /Helvetica /Helvetica-Bold /Helvetica-Oblique /Helvetica-BoldOblique /Times-Roman /Times-Bold /Times-Italic /Times-BoldItalic /Symbol /ZapfDingbats /Arial] >> setdistillerparams";
const GS_PRE_PDFMARK: &str = "/originalpdfmark { //pdfmark } bind def /pdfmark { { { counttomark pop } stopped { /pdfmark errordict /unmatchedmark get exec stop } if dup type /nametype ne { /pdfmark errordict /typecheck get exec stop } if dup /DOCINFO eq { (Skipping DOCINFO pdfmark\\n) print cleartomark exit } if originalpdfmark exit } loop } def";

fn gs_args(input: &str, output: &str, lossy: bool, dpi: i32) -> Vec<String> {
    let clamped = dpi.clamp(PDF_DPI_MIN, PDF_DPI_MAX);
    let downsample = clamped < PDF_DPI_NO_DOWNSAMPLE;
    let mut args: Vec<String> = gs_base_args().into_iter().map(String::from).collect();
    args.extend(gs_resolution_args(clamped));
    if lossy {
        args.extend(gs_lossy_args(downsample));
    } else {
        args.extend(gs_lossless_args(downsample));
    }
    args.extend(["-sDEVICE=pdfwrite".into(), "-o".into(), output.to_string()]);
    // GS_PRE_ARGS
    args.extend([
        "-c".into(),
        GS_PRE_DISTILLER.into(),
        "-f".into(),
        "-c".into(),
        GS_PRE_PDFMARK.into(),
        "-f".into(),
    ]);
    args.push(input.to_string());
    // GS_POST_ARGS
    args.extend([
        "-c".into(),
        "/pdfmark { originalpdfmark } bind def".into(),
        "-f".into(),
        "-c".into(),
        "[ /Producer () /ModDate () /CreationDate () /DOCINFO pdfmark".into(),
        "-f".into(),
    ]);
    args
}

// ---------------------------------------------------------------------------
// Non-destructive crop / uncrop (sets or removes the page /CropBox via lopdf)
// ---------------------------------------------------------------------------

fn obj_num(doc: &lopdf::Document, o: &lopdf::Object) -> Option<f64> {
    match o {
        lopdf::Object::Integer(i) => Some(*i as f64),
        lopdf::Object::Real(r) => Some(*r as f64),
        lopdf::Object::Reference(id) => doc.get_object(*id).ok().and_then(|x| obj_num(doc, x)),
        _ => None,
    }
}

fn rect_of(doc: &lopdf::Document, o: &lopdf::Object) -> Option<[f64; 4]> {
    let arr = o.as_array().ok()?;
    if arr.len() != 4 {
        return None;
    }
    Some([
        obj_num(doc, &arr[0])?,
        obj_num(doc, &arr[1])?,
        obj_num(doc, &arr[2])?,
        obj_num(doc, &arr[3])?,
    ])
}

fn resolve_mediabox(doc: &lopdf::Document, page_id: (u32, u16)) -> [f64; 4] {
    let mut id = page_id;
    for _ in 0..32 {
        let Ok(obj) = doc.get_object(id) else { break };
        let Ok(dict) = obj.as_dict() else { break };
        if let Ok(mb) = dict.get(b"MediaBox") {
            if let Some(r) = rect_of(doc, mb) {
                return r;
            }
        }
        match dict.get(b"Parent") {
            Ok(lopdf::Object::Reference(pid)) => id = *pid,
            _ => break,
        }
    }
    [0.0, 0.0, 612.0, 792.0] // US Letter fallback
}

fn crop_box_for(media: [f64; 4], aspect: (f64, f64)) -> [f64; 4] {
    let [x0, y0, x1, y1] = media;
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0.0 || h <= 0.0 || aspect.0 <= 0.0 || aspect.1 <= 0.0 {
        return media;
    }
    let r = aspect.0 / aspect.1;
    if w / h > r {
        let nw = h * r;
        let nx0 = x0 + (w - nw) / 2.0;
        [nx0, y0, nx0 + nw, y1]
    } else {
        let nh = w / r;
        let ny0 = y0 + (h - nh) / 2.0;
        [x0, ny0, x1, ny0 + nh]
    }
}

/// Crop every page to `aspect` (e.g. (16.0, 9.0)) by setting the `/CropBox`.
/// Non-destructive: the original content is preserved and [`uncrop`] reverts it.
pub fn crop(path: &Path, output: &Path, aspect: (f64, f64)) -> Result<(), OptimiseError> {
    let mut doc =
        lopdf::Document::load(path).map_err(|e| OptimiseError::Other(format!("pdf load: {e}")))?;
    let pages: Vec<(u32, (u32, u16))> = doc.get_pages().into_iter().collect();
    for (_n, id) in pages {
        let media = resolve_mediabox(&doc, id);
        let cb = crop_box_for(media, aspect);
        if let Ok(dict) = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()) {
            dict.set(
                "CropBox",
                lopdf::Object::Array(cb.iter().map(|v| lopdf::Object::Real(*v as f32)).collect()),
            );
        }
    }
    doc.save(output)
        .map_err(|e| OptimiseError::Other(format!("pdf save: {e}")))?;
    Ok(())
}

/// Remove the `/CropBox` from every page, reverting a non-destructive crop.
pub fn uncrop(path: &Path, output: &Path) -> Result<(), OptimiseError> {
    let mut doc =
        lopdf::Document::load(path).map_err(|e| OptimiseError::Other(format!("pdf load: {e}")))?;
    let pages: Vec<(u32, (u32, u16))> = doc.get_pages().into_iter().collect();
    for (_n, id) in pages {
        if let Ok(dict) = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()) {
            dict.remove(b"CropBox");
        }
    }
    doc.save(output)
        .map_err(|e| OptimiseError::Other(format!("pdf save: {e}")))?;
    Ok(())
}

/// Render each PDF page to an image (`png` or `jpeg`) in `out_dir` via ghostscript.
/// Returns the generated image paths.
pub fn extract_pages(
    path: &Path,
    out_dir: &Path,
    format: &str,
    dpi: i32,
) -> Result<Vec<std::path::PathBuf>, OptimiseError> {
    std::fs::create_dir_all(out_dir)?;
    let (device, ext) = match format.to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => ("jpeg", "jpg"),
        _ => ("png16m", "png"),
    };
    let stem = crate::result::file_stem_lossy(path);
    let pattern = out_dir.join(format!("{stem}-%03d.{ext}"));
    tools::run(
        Tool::Ghostscript,
        [
            "-dNOPAUSE",
            "-dBATCH",
            "-dSAFER",
            "-dQUIET",
            &format!("-sDEVICE={device}"),
            &format!("-r{}", dpi.clamp(36, 600)),
            &format!("-sOutputFile={}", pattern.display()),
            &path.display().to_string(),
        ],
    )?;
    let mut out: Vec<_> = std::fs::read_dir(out_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(ext))
        .collect();
    out.sort();
    Ok(out)
}

/// Optimise a PDF. `dpi` controls downsampling; `None` means no downsample (300).
pub fn optimise(
    path: &Path,
    options: &OptimiseOptions,
    dpi: Option<i32>,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let old_size = file_size(path);
    let effective_dpi = dpi.unwrap_or(PDF_DPI_NO_DOWNSAMPLE);
    let lossy = effective_dpi < PDF_DPI_NO_DOWNSAMPLE;

    let tmp = TempDir::new()?;
    let temp_out = tmp.path().join(crate::result::file_name_lossy(path));

    let args = gs_args(
        &path.display().to_string(),
        &temp_out.display().to_string(),
        lossy,
        effective_dpi,
    );
    tools::run_with_retries(Tool::Ghostscript, &args, 3)?;

    let new_size = file_size(&temp_out);

    if !options.allow_larger && new_size >= old_size {
        return Ok(OptimisationResult {
            kind: MediaKind::Pdf,
            source: path.to_path_buf(),
            output: path.to_path_buf(),
            backup: None,
            old_size,
            new_size: old_size,
            aggressive: lossy,
        });
    }

    let backup = if options.backup && options.output.is_none() {
        Some(backup_file(path)?)
    } else {
        None
    };

    let dest = options.output.clone().unwrap_or_else(|| path.to_path_buf());
    std::fs::copy(&temp_out, &dest)?;
    if options.preserve_dates {
        copy_dates(path, &dest);
    }

    Ok(OptimisationResult {
        kind: MediaKind::Pdf,
        source: path.to_path_buf(),
        output: dest,
        backup,
        old_size,
        new_size,
        aggressive: lossy,
    })
}
