//! Text recognition in images (OCR), so screenshots and copied images can be
//! searched by the words in them. macOS uses Apple's Vision framework: on
//! device, no network, many languages. Other platforms report it unsupported.

use std::path::Path;

/// Recognise the text in an image file. Lines come out in reading order,
/// separated by newlines; an image without text gives an empty string.
pub fn recognize_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    recognize(&bytes)
}

/// Recognise the text in encoded image bytes (PNG, JPEG, TIFF, HEIC, …).
#[cfg(target_os = "macos")]
pub fn recognize(bytes: &[u8]) -> Result<String, String> {
    use objc2::AllocAnyThread;
    use objc2_foundation::{NSArray, NSData, NSDictionary};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };

    objc2::rc::autoreleasepool(|_| {
        let data = NSData::with_bytes(bytes);
        let handler = VNImageRequestHandler::initWithData_options(
            VNImageRequestHandler::alloc(),
            &data,
            &NSDictionary::new(),
        );
        // SAFETY: a plain request with no completion handler.
        let request = unsafe { VNRecognizeTextRequest::init(VNRecognizeTextRequest::alloc()) };
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        request.setAutomaticallyDetectsLanguage(true);

        let as_request: &VNRequest = &request;
        handler
            .performRequests_error(&NSArray::from_slice(&[as_request]))
            .map_err(|e| e.localizedDescription().to_string())?;

        let lines: Vec<String> = request
            .results()
            .map(|observations| {
                observations
                    .iter()
                    .filter_map(|o| o.topCandidates(1).firstObject())
                    .map(|t| t.string().to_string())
                    .collect()
            })
            .unwrap_or_default();
        Ok(lines.join("\n"))
    })
}

#[cfg(not(target_os = "macos"))]
pub fn recognize(_bytes: &[u8]) -> Result<String, String> {
    Err("text recognition is only available on macOS".into())
}

/// Whether [`recognize`] works on this platform.
pub fn available() -> bool {
    cfg!(target_os = "macos")
}
