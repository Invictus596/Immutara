//! Local face-crop generation.
//!
//! The pipeline derives the search input from the analysis stage: given the
//! selected-face bounding box, it crops the evidence image with a small
//! padding so the region submitted to the reverse-image-search provider
//! contains the face (and a bit of context) — no background, no full image.
//!
//! Only the re-encoded crop bytes leave the machine (to the search
//! provider). The crop is plain image data; it is never a biometric
//! embedding.

use image::GenericImageView;

use immutara_core::ImmutaraError;
use immutara_core::domain::analysis::BoundingBox;
use immutara_core::domain::search::FaceCrop;

/// Fraction of the face-box extent added on each side when cropping.
const PADDING_RATIO: f64 = 0.25;
/// Base JPEG quality used when re-encoding the crop.
const JPEG_QUALITY: u8 = 90;
/// How much to scale down each iteration when the crop is too large.
const DOWNSAMPLE_FACTOR: f64 = 0.75;

/// Generate a re-encoded JPEG face-crop from the evidence bytes.
///
/// The bounding box is padded by [`PADDING_RATIO`] on each side, clamped to
/// the image bounds, cropped, and re-encoded. If the encoded bytes exceed
/// `max_upload_bytes` the crop is progressively downsampled until it fits.
/// Returns an error when the input cannot be decoded as an image.
pub fn generate_face_crop(
    image_bytes: &[u8],
    bounding_box: BoundingBox,
    max_upload_bytes: usize,
) -> Result<FaceCrop, ImmutaraError> {
    let mut image = image::load_from_memory(image_bytes).map_err(|e| ImmutaraError::Provider {
        provider: "face_crop".to_string(),
        message: format!("cannot decode evidence as an image to crop the face: {e}"),
    })?;

    let (img_w, img_h) = image.dimensions();
    let padded = padded_box(bounding_box, img_w, img_h);
    let mut cropped = image.crop(padded.x, padded.y, padded.width, padded.height);

    let mut bytes = encode_jpeg(&cropped, JPEG_QUALITY);
    while bytes.len() > max_upload_bytes {
        let (w, h) = cropped.dimensions();
        let new_w = ((w as f64 * DOWNSAMPLE_FACTOR) as u32).max(1);
        let new_h = ((h as f64 * DOWNSAMPLE_FACTOR) as u32).max(1);
        cropped = cropped.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle);
        bytes = encode_jpeg(&cropped, JPEG_QUALITY);
    }

    Ok(FaceCrop {
        image_bytes: bytes,
        mime_type: "image/jpeg".to_string(),
        bounding_box: padded,
    })
}

/// Expand `bbox` by `PADDING_RATIO` on each side and clamp to the image.
fn padded_box(bbox: BoundingBox, img_w: u32, img_h: u32) -> BoundingBox {
    let pad_x = (bbox.width as f64 * PADDING_RATIO) as u32;
    let pad_y = (bbox.height as f64 * PADDING_RATIO) as u32;

    let x = bbox.x.saturating_sub(pad_x);
    let y = bbox.y.saturating_sub(pad_y);

    // Clamp extent to the right/bottom edges.
    let width = (bbox.width + pad_x * 2).min(img_w.saturating_sub(x)).max(1);
    let height = (bbox.height + pad_y * 2)
        .min(img_h.saturating_sub(y))
        .max(1);

    BoundingBox {
        x,
        y,
        width,
        height,
    }
}

/// Re-encode an image as JPEG bytes at the given quality.
fn encode_jpeg(image: &image::DynamicImage, quality: u8) -> Vec<u8> {
    let mut buf = Vec::new();
    let rgb = image.to_rgb8();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    // Encoding into a Vec cannot fail; ignore the result type.
    let _ = encoder.encode_image(&rgb);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 20x20 base64 is unnecessary; build a tiny JPEG via the image crate.
    fn tiny_jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([120, 130, 140]));
        let mut buf = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
        enc.encode_image(&img).unwrap();
        buf
    }

    #[test]
    fn crop_is_a_valid_smaller_image() {
        let bytes = tiny_jpeg(200, 200);
        let crop = generate_face_crop(
            &bytes,
            BoundingBox {
                x: 50,
                y: 50,
                width: 100,
                height: 100,
            },
            500_000,
        )
        .unwrap();
        // 100x100 box padded by 25% = 150x150, clamped to 200x200 bounds.
        assert_eq!(
            crop.bounding_box,
            BoundingBox {
                x: 25,
                y: 25,
                width: 150,
                height: 150
            }
        );
        assert_eq!(crop.mime_type, "image/jpeg");
        assert!(!crop.image_bytes.is_empty());
        // Re-decode to verify it's a real JPEG.
        let decoded = image::load_from_memory(&crop.image_bytes).unwrap();
        assert_eq!(decoded.width(), 150);
        assert_eq!(decoded.height(), 150);
    }

    #[test]
    fn crop_clamps_to_image_bounds() {
        let bytes = tiny_jpeg(100, 100);
        // Face box hugging the top-left corner; padding would overflow.
        let crop = generate_face_crop(
            &bytes,
            BoundingBox {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            500_000,
        )
        .unwrap();
        assert_eq!(crop.bounding_box.x, 0);
        assert_eq!(crop.bounding_box.y, 0);
        // 10 wide padded by 25% (2.5 -> 2) * 2 sides = 14, clamped to 100 wide.
        assert!(crop.bounding_box.width <= 100);
        assert!(crop.bounding_box.height <= 100);
    }

    #[test]
    fn crop_respects_upload_size_cap() {
        let bytes = tiny_jpeg(1200, 900);
        let crop = generate_face_crop(
            &bytes,
            BoundingBox {
                x: 100,
                y: 100,
                width: 800,
                height: 500,
            },
            2_000,
        )
        .unwrap();
        assert!(
            crop.image_bytes.len() <= 2_000,
            "crop {} bytes must fit cap",
            crop.image_bytes.len()
        );
    }

    #[test]
    fn non_image_input_is_error() {
        let err = generate_face_crop(
            b"not an image",
            BoundingBox {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            500_000,
        )
        .unwrap_err();
        assert!(matches!(err, ImmutaraError::Provider { .. }));
    }
}
