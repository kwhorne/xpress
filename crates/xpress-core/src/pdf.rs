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
    let temp_out = tmp.path().join(path.file_name().unwrap());

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
