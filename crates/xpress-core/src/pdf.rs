//! Pure-Rust PDF optimisation and non-destructive crop.
//!
//! Optimisation recompresses embedded JPEG images (via the image engine) and
//! losslessly re-compresses streams with `lopdf` — no external tool. Only
//! `extract-pages` (rendering pages to images) still uses ghostscript.

use std::path::Path;

use tempfile::TempDir;

use crate::filetype::MediaKind;
use crate::result::{
    backup_file, copy_dates, file_size, OptimisationResult, OptimiseError, OptimiseOptions,
};
use crate::tools::{self, Tool};

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
/// Whether a stream dictionary is a JPEG (DCTDecode) image XObject.
fn is_dct_image(dict: &lopdf::Dictionary) -> bool {
    let is_image = matches!(dict.get(b"Subtype"), Ok(lopdf::Object::Name(n)) if n == b"Image");
    if !is_image {
        return false;
    }
    match dict.get(b"Filter") {
        Ok(lopdf::Object::Name(n)) => n == b"DCTDecode",
        Ok(lopdf::Object::Array(a)) => a
            .iter()
            .any(|o| matches!(o, lopdf::Object::Name(n) if n == b"DCTDecode")),
        _ => false,
    }
}

/// Optimise a PDF in pure Rust: re-compress embedded JPEG (DCTDecode) images at
/// the target quality and losslessly re-compress content streams. `dpi` is
/// currently advisory (image downsampling is a future addition).
pub fn optimise(
    path: &Path,
    options: &OptimiseOptions,
    _dpi: Option<i32>,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let old_size = file_size(path);
    let cq = options.compression;
    let quality = cq.jpeg_max_quality().clamp(1, 100) as u8;

    let mut doc =
        lopdf::Document::load(path).map_err(|e| OptimiseError::Other(format!("pdf load: {e}")))?;

    // Re-encode embedded JPEG images at the target quality (keep the smaller one).
    let ids: Vec<(u32, u16)> = doc
        .objects
        .iter()
        .filter_map(|(id, obj)| match obj {
            lopdf::Object::Stream(s) if is_dct_image(&s.dict) => Some(*id),
            _ => None,
        })
        .collect();

    for id in ids {
        if let Ok(lopdf::Object::Stream(stream)) = doc.get_object_mut(id) {
            let original = std::mem::take(&mut stream.content);
            let recompressed = match image::load_from_memory(&original) {
                Ok(img) => {
                    let mut buf = Vec::new();
                    let mut enc =
                        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
                    if enc.encode_image(&img).is_ok()
                        && !buf.is_empty()
                        && buf.len() < original.len()
                    {
                        Some(buf)
                    } else {
                        None
                    }
                }
                Err(_) => None, // CMYK/JPX/unsupported: leave untouched
            };
            match recompressed {
                Some(buf) => {
                    stream.dict.set("Length", buf.len() as i64);
                    stream.set_content(buf);
                }
                None => stream.set_content(original),
            }
        }
    }

    // Lossless structural gains: drop orphans and Flate-compress plain streams.
    let _ = doc.prune_objects();
    doc.compress();

    let tmp = TempDir::new()?;
    let temp_out = tmp.path().join(crate::result::file_name_lossy(path));
    doc.save(&temp_out)
        .map_err(|e| OptimiseError::Other(format!("pdf save: {e}")))?;

    let new_size = file_size(&temp_out);
    let aggressive = cq.image_is_aggressive();

    if !options.allow_larger && (new_size == 0 || new_size >= old_size) {
        return Ok(OptimisationResult {
            kind: MediaKind::Pdf,
            source: path.to_path_buf(),
            output: path.to_path_buf(),
            backup: None,
            old_size,
            new_size: old_size,
            aggressive,
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
        aggressive,
    })
}
