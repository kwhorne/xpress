//! Text recognition in images (OCR), so screenshots and copied images can be
//! searched by the words in them. macOS uses Apple's Vision framework: on
//! device, no network, many languages. Other platforms report it unsupported.

use std::path::Path;

/// Recognise the text in an image file. Lines come out in reading order,
/// separated by newlines; an image without text gives an empty string.
#[cfg(target_os = "macos")]
pub fn recognize_file(path: &Path) -> Result<String, String> {
    use objc2::AllocAnyThread;
    use objc2_foundation::{NSDictionary, NSString, NSURL};
    use objc2_vision::VNImageRequestHandler;
    // Vision reads the file itself: no copy of it in our memory, let alone two.
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    let path = path.to_str().ok_or("the file name is not valid UTF-8")?;
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        // SAFETY: an empty options dictionary.
        let handler = unsafe {
            VNImageRequestHandler::initWithURL_options(
                VNImageRequestHandler::alloc(),
                &url,
                &NSDictionary::new(),
            )
        };
        perform(&handler)
    })
}

/// Recognise the text in an image file (see [`recognize`]).
#[cfg(not(target_os = "macos"))]
pub fn recognize_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    recognize(&bytes)
}

/// Recognise the text in encoded image bytes (PNG, JPEG, TIFF, HEIC, …).
#[cfg(target_os = "macos")]
pub fn recognize(bytes: &[u8]) -> Result<String, String> {
    use objc2::AllocAnyThread;
    use objc2_foundation::{NSData, NSDictionary};
    use objc2_vision::VNImageRequestHandler;

    objc2::rc::autoreleasepool(|_| {
        let data = NSData::with_bytes(bytes);
        let handler = VNImageRequestHandler::initWithData_options(
            VNImageRequestHandler::alloc(),
            &data,
            &NSDictionary::new(),
        );
        perform(&handler)
    })
}

/// Run accurate text recognition through `handler`.
#[cfg(target_os = "macos")]
fn perform(handler: &objc2_vision::VNImageRequestHandler) -> Result<String, String> {
    use objc2::AllocAnyThread;
    use objc2_foundation::NSArray;
    use objc2_vision::{VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel};
    {
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
    }
}

#[cfg(not(target_os = "macos"))]
pub fn recognize(_bytes: &[u8]) -> Result<String, String> {
    Err("text recognition is only available on macOS".into())
}

/// Whether [`recognize`] works on this platform.
pub fn available() -> bool {
    cfg!(target_os = "macos")
}
