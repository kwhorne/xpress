//! Pure-Rust PDF optimisation and non-destructive crop.
//!
//! Optimisation recompresses embedded JPEG images (via the image engine),
//! optionally downsampling them to a target DPI at their drawn size, and
//! losslessly re-compresses streams with `lopdf` — no external tool. Only
//! `extract-pages` (rendering pages to images) still uses ghostscript.

use std::collections::HashMap;
use std::path::Path;

use tempfile::TempDir;

use crate::filetype::MediaKind;
use crate::result::{
    file_size, finish, OptimisationResult, OptimiseError, OptimiseOptions, Placement,
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

type ObjectId = (u32, u16);

/// If `dict` is a JPEG image XObject that can be decoded and re-encoded without
/// changing its colours, the number of colour channels (1 or 3). Only a plain
/// `DCTDecode` stream in DeviceGray/DeviceRGB (or a 1-/3-component ICCBased
/// space) at 8 bits with no `/Decode` remapping qualifies; CMYK, Lab, Indexed,
/// Separation/DeviceN or chained filters are left untouched, since the decoder
/// would silently convert them to RGB while the PDF still declares the old
/// colour space.
fn recompressible_channels(doc: &lopdf::Document, dict: &lopdf::Dictionary) -> Option<u8> {
    use lopdf::Object;
    if !matches!(dict.get(b"Subtype"), Ok(Object::Name(n)) if n == b"Image") {
        return None;
    }
    let plain_dct = match dict.get(b"Filter").ok()? {
        Object::Name(n) => n == b"DCTDecode",
        Object::Array(a) => a.len() == 1 && matches!(&a[0], Object::Name(n) if n == b"DCTDecode"),
        _ => false,
    };
    if !plain_dct || dict.has(b"Decode") || dict.has(b"DecodeParms") {
        return None;
    }
    if !matches!(
        dict.get(b"BitsPerComponent"),
        Ok(Object::Integer(8)) | Err(_)
    ) {
        return None;
    }
    let resolve = |o: &Object| -> Option<Object> {
        match o {
            Object::Reference(id) => doc.get_object(*id).ok().cloned(),
            other => Some(other.clone()),
        }
    };
    match resolve(dict.get(b"ColorSpace").ok()?)? {
        Object::Name(n) if n == b"DeviceRGB" => Some(3),
        Object::Name(n) if n == b"DeviceGray" => Some(1),
        Object::Array(a)
            if a.len() == 2 && matches!(&a[0], Object::Name(n) if n == b"ICCBased") =>
        {
            let Object::Stream(icc) = resolve(&a[1])? else {
                return None;
            };
            match icc.dict.get(b"N") {
                Ok(Object::Integer(1)) => Some(1),
                Ok(Object::Integer(3)) => Some(3),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Multiply two PDF matrices `[a b c d e f]` (`m` applied first).
fn mat_mul(m: [f64; 6], n: [f64; 6]) -> [f64; 6] {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

/// The page's XObject resource names -> object ids (inherited resources too).
fn xobject_names(doc: &lopdf::Document, page_id: ObjectId) -> HashMap<Vec<u8>, ObjectId> {
    let mut names = HashMap::new();
    let Ok((inline, ids)) = doc.get_page_resources(page_id) else {
        return names;
    };
    let dicts = inline
        .into_iter()
        .chain(ids.iter().filter_map(|id| doc.get_dictionary(*id).ok()));
    for res in dicts {
        let xobjects = match res.get(b"XObject") {
            Ok(lopdf::Object::Dictionary(d)) => Some(d),
            Ok(lopdf::Object::Reference(id)) => doc.get_dictionary(*id).ok(),
            _ => None,
        };
        for (name, obj) in xobjects.into_iter().flat_map(|d| d.iter()) {
            if let lopdf::Object::Reference(id) = obj {
                names.entry(name.clone()).or_insert(*id);
            }
        }
    }
    names
}

/// The largest size (width, height in points) each image XObject is drawn at,
/// found by tracking the transformation matrix through every page's content
/// stream up to its `Do`. Images drawn only inside forms/patterns aren't found.
fn placed_sizes(doc: &lopdf::Document) -> HashMap<ObjectId, (f64, f64)> {
    let mut sizes: HashMap<ObjectId, (f64, f64)> = HashMap::new();
    for page_id in doc.get_pages().into_values() {
        let names = xobject_names(doc, page_id);
        let bytes = doc.get_page_content(page_id);
        let Ok(content) = lopdf::content::Content::decode(&bytes) else {
            continue;
        };
        let mut ctm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let mut stack = Vec::new();
        for op in content.operations {
            match op.operator.as_str() {
                "q" => stack.push(ctm),
                "Q" => ctm = stack.pop().unwrap_or(ctm),
                "cm" => {
                    let v: Vec<f64> = op
                        .operands
                        .iter()
                        .filter_map(|o| o.as_float().ok().map(f64::from))
                        .collect();
                    if let Ok(m) = <[f64; 6]>::try_from(v) {
                        ctm = mat_mul(m, ctm);
                    }
                }
                "Do" => {
                    let id = op
                        .operands
                        .first()
                        .and_then(|o| o.as_name().ok())
                        .and_then(|n| names.get(n));
                    if let Some(id) = id {
                        let w = ctm[0].hypot(ctm[1]);
                        let h = ctm[2].hypot(ctm[3]);
                        let e = sizes.entry(*id).or_insert((0.0, 0.0));
                        *e = (e.0.max(w), e.1.max(h));
                    }
                }
                _ => {}
            }
        }
    }
    sizes
}

/// The longest page edge in the document, in points.
fn longest_page_edge(doc: &lopdf::Document) -> f64 {
    doc.get_pages()
        .into_values()
        .map(|id| {
            let [x0, y0, x1, y1] = resolve_mediabox(doc, id);
            (x1 - x0).abs().max((y1 - y0).abs())
        })
        .fold(0.0, f64::max)
}

/// The pixel size to downsample a `w`x`h` image to so it is no sharper than
/// `dpi` where it is drawn (or, if its placement is unknown, even when drawn
/// across the longest page edge). `None` when it is already within the limit.
fn downsampled_size(
    w: u32,
    h: u32,
    dpi: f64,
    placed: Option<(f64, f64)>,
    page_edge: f64,
) -> Option<(u32, u32)> {
    let scale = match placed {
        Some((pw, ph)) if pw > 0.0 && ph > 0.0 => {
            (pw / 72.0 * dpi / w as f64).max(ph / 72.0 * dpi / h as f64)
        }
        _ if page_edge > 0.0 => page_edge / 72.0 * dpi / w.max(h) as f64,
        _ => return None,
    };
    // Leave a little slack so near-target images aren't resampled for nothing.
    if scale >= 0.9 {
        return None;
    }
    let nw = ((w as f64 * scale).round() as u32).max(1);
    let nh = ((h as f64 * scale).round() as u32).max(1);
    Some((nw, nh))
}

/// Optimise a PDF in pure Rust: re-compress embedded JPEG images at the target
/// quality (only where that can't change their colours), optionally downsample
/// them to `dpi` at their drawn size, and losslessly re-compress streams.
pub fn optimise(
    path: &Path,
    options: &OptimiseOptions,
    dpi: Option<i32>,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let old_size = file_size(path);
    let cq = options.compression;
    let quality = cq.jpeg_max_quality().clamp(1, 100) as u8;
    let dpi = dpi.map(|d| d.clamp(36, 600) as f64);

    let mut doc =
        lopdf::Document::load(path).map_err(|e| OptimiseError::Other(format!("pdf load: {e}")))?;

    let placed = if dpi.is_some() {
        placed_sizes(&doc)
    } else {
        HashMap::new()
    };
    let page_edge = longest_page_edge(&doc);

    let targets: Vec<(ObjectId, u8)> = doc
        .objects
        .iter()
        .filter_map(|(id, obj)| match obj {
            lopdf::Object::Stream(s) => recompressible_channels(&doc, &s.dict).map(|c| (*id, c)),
            _ => None,
        })
        .collect();

    for (id, channels) in targets {
        let Ok(lopdf::Object::Stream(stream)) = doc.get_object_mut(id) else {
            continue;
        };
        let Ok(img) = image::load_from_memory(&stream.content) else {
            continue;
        };
        // Keep the declared channel count. (The decoder may return a grayscale
        // JPEG as RGB; accept that only if every pixel really is gray.)
        let img = match (channels, img) {
            (1, img @ image::DynamicImage::ImageLuma8(_)) => img,
            (1, image::DynamicImage::ImageRgb8(rgb))
                if rgb.pixels().all(|p| p.0[0] == p.0[1] && p.0[1] == p.0[2]) =>
            {
                image::DynamicImage::ImageLuma8(image::GrayImage::from_fn(
                    rgb.width(),
                    rgb.height(),
                    |x, y| image::Luma([rgb.get_pixel(x, y).0[0]]),
                ))
            }
            (3, img @ image::DynamicImage::ImageRgb8(_)) => img,
            _ => continue,
        };
        let resize = dpi.and_then(|d| {
            downsampled_size(
                img.width(),
                img.height(),
                d,
                placed.get(&id).copied(),
                page_edge,
            )
        });
        let img = match resize {
            Some((w, h)) => img.resize_exact(w, h, image::imageops::FilterType::Lanczos3),
            None => img,
        };
        let mut buf = Vec::new();
        // `write_with_encoder` keeps the colour type (a gray image stays a
        // 1-component JPEG); `encode_image` would always write RGB.
        let encoded = img
            .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut buf, quality,
            ))
            .is_ok();
        // A downsampled image is always used; a same-size one only if smaller.
        if !encoded || buf.is_empty() || (resize.is_none() && buf.len() >= stream.content.len()) {
            continue;
        }
        if resize.is_some() {
            stream.dict.set("Width", img.width() as i64);
            stream.dict.set("Height", img.height() as i64);
        }
        stream.dict.set("Length", buf.len() as i64);
        stream.set_content(buf);
    }

    // Lossless structural gains: drop orphans and Flate-compress plain streams.
    let _ = doc.prune_objects();
    doc.compress();

    let tmp = TempDir::new()?;
    let temp_out = tmp.path().join(crate::result::file_name_lossy(path));
    doc.save(&temp_out)
        .map_err(|e| OptimiseError::Other(format!("pdf save: {e}")))?;

    finish(
        MediaKind::Pdf,
        path,
        &temp_out,
        path.to_path_buf(),
        old_size,
        cq.image_is_aggressive(),
        options,
        Placement {
            size_guard: true,
            backup: true,
            replace_source: false,
        },
    )
}
