/// Maximum image size in bytes (5 MiB).
/// Anthropic Messages API limits base64-encoded images to roughly 5 MB.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

pub fn max_base64_image_len() -> usize {
    MAX_IMAGE_BYTES.div_ceil(3) * 4
}

pub fn reject_oversized_base64_image(data: &str) -> Result<(), String> {
    let max_encoded_len = max_base64_image_len();
    if data.len() > max_encoded_len {
        return Err(format!(
            "Image too large: encoded length {} exceeds max encoded length {}",
            data.len(),
            max_encoded_len
        ));
    }
    Ok(())
}

pub fn validate_image_bytes(bytes: &[u8]) -> Result<&'static str, String> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "Image too large: {} bytes (max {} bytes)",
            bytes.len(),
            MAX_IMAGE_BYTES
        ));
    }

    detect_image_mime(bytes).ok_or_else(|| "Unsupported image format".to_string())
}

pub fn validate_image_bytes_for_media_type(
    bytes: &[u8],
    media_type: &str,
) -> Result<&'static str, String> {
    let detected = validate_image_bytes(bytes)?;
    if detected != media_type {
        return Err(format!(
            "Image media type mismatch: declared {media_type}, detected {detected}"
        ));
    }
    Ok(detected)
}

/// Detect MIME type from magic bytes.
pub fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 {
        return None;
    }
    // JPEG: FF D8 FF
    if bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some("image/jpeg");
    }
    // PNG: 89 50 4E 47
    if bytes[0] == 0x89 && bytes[1] == 0x50 && bytes[2] == 0x4E && bytes[3] == 0x47 {
        return Some("image/png");
    }
    // GIF: 47 49 46 38
    if bytes[0] == 0x47 && bytes[1] == 0x49 && bytes[2] == 0x46 && bytes[3] == 0x38 {
        return Some("image/gif");
    }
    // WebP: RIFF....WEBP
    if bytes.len() >= 12
        && bytes[0] == 0x52
        && bytes[1] == 0x49
        && bytes[2] == 0x46
        && bytes[3] == 0x46
        && bytes[8] == 0x57
        && bytes[9] == 0x45
        && bytes[10] == 0x42
        && bytes[11] == 0x50
    {
        return Some("image/webp");
    }
    None
}
