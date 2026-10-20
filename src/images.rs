//! Images attached to messages: loaded from files, the clipboard or drops,
//! scaled down to what the API accepts, base64-encoded.

use crate::chat::Attachment;
use crate::claude::protocol::ImageBlock;
use base64::Engine;
use std::path::Path;

/// The API's per-image limits (bytes after decoding, longest side).
const MAX_BYTES: usize = 3_750_000;
const MAX_SIDE: u32 = 4096;

pub fn is_image(path: &Path) -> bool {
    matches!(
        path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

fn media_type(path: &Path) -> &'static str {
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

fn encode_png(img: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Keep the original bytes when they're within limits, else re-encode a
/// scaled-down PNG.
fn prepare(bytes: Vec<u8>, media: &str) -> Result<(Vec<u8>, String, image::DynamicImage), String> {
    let img = image::load_from_memory(&bytes).map_err(|e| format!("not an image: {e}"))?;
    if bytes.len() <= MAX_BYTES && img.width() <= MAX_SIDE && img.height() <= MAX_SIDE {
        return Ok((bytes, media.to_string(), img));
    }
    let mut side = MAX_SIDE;
    loop {
        let scaled = img.resize(side, side, image::imageops::FilterType::Triangle);
        let png = encode_png(&scaled)?;
        if png.len() <= MAX_BYTES || side <= 512 {
            return Ok((png, "image/png".into(), scaled));
        }
        side = side * 3 / 4;
    }
}

fn attachment(name: String, bytes: Vec<u8>, media: &str, ctx: &egui::Context) -> Result<Attachment, String> {
    let (bytes, media, img) = prepare(bytes, media)?;
    let thumb = img.thumbnail(128, 128).to_rgba8();
    let color = egui::ColorImage::from_rgba_unmultiplied([thumb.width() as usize, thumb.height() as usize], thumb.as_raw());
    let texture = ctx.load_texture(format!("attach-{name}-{}", bytes.len()), color, egui::TextureOptions::LINEAR);
    Ok(Attachment {
        name,
        image: ImageBlock { media_type: media, base64: base64::engine::general_purpose::STANDARD.encode(&bytes) },
        texture: Some(texture),
    })
}

pub fn from_file(path: &Path, ctx: &egui::Context) -> Result<Attachment, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
    attachment(name, bytes, media_type(path), ctx)
}

/// An image on the clipboard, if any.
pub fn from_clipboard(ctx: &egui::Context) -> Option<Result<Attachment, String>> {
    let mut cb = arboard::Clipboard::new().ok()?;
    let img = cb.get_image().ok()?;
    let rgba = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())?;
    let png = encode_png(&image::DynamicImage::ImageRgba8(rgba));
    Some(png.and_then(|png| attachment("Pasted image".into(), png, "image/png", ctx)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_images_are_scaled() {
        let big = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(5000, 100, image::Rgba([1, 2, 3, 255])));
        let png = encode_png(&big).unwrap();
        let (bytes, media, img) = prepare(png, "image/png").unwrap();
        assert!(img.width() <= MAX_SIDE);
        assert_eq!(media, "image/png");
        assert!(bytes.len() <= MAX_BYTES);
        assert!(prepare(b"nope".to_vec(), "image/png").is_err());
        assert!(is_image(Path::new("a/b.JPG")));
        assert!(!is_image(Path::new("a/b.txt")));
    }
}
