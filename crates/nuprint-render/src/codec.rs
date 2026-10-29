//! Source decoding (pictures), format sniffing and the 1-bit PNG preview/export codec.

use std::io::Cursor;

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use nuprint_protocol::MonoBitmap;

use crate::raster::{self, composite_on_white};
use crate::{ImageParams, MAX_DECODED_PIXELS, MAX_SOURCE_BYTES, RenderError, RenderOutput, RenderTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Png,
    Jpeg,
    Bmp,
    Gif,
    Webp,
    Tiff,
    Pdf,
}

impl SourceFormat {
    fn image_format(self) -> Option<ImageFormat> {
        Some(match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Bmp => ImageFormat::Bmp,
            Self::Gif => ImageFormat::Gif,
            Self::Webp => ImageFormat::WebP,
            Self::Tiff => ImageFormat::Tiff,
            Self::Pdf => return None,
        })
    }
}

/// Identify a file by its leading bytes, never by its extension.
pub fn sniff_format(bytes: &[u8]) -> Option<SourceFormat> {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    if starts(b"\x89PNG\r\n\x1a\n") {
        Some(SourceFormat::Png)
    } else if starts(&[0xFF, 0xD8, 0xFF]) {
        Some(SourceFormat::Jpeg)
    } else if starts(b"BM") {
        Some(SourceFormat::Bmp)
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some(SourceFormat::Gif)
    } else if bytes.len() >= 12 && starts(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some(SourceFormat::Webp)
    } else if starts(b"II*\0") || starts(b"MM\0*") {
        Some(SourceFormat::Tiff)
    } else if bytes[..bytes.len().min(1024)].windows(5).any(|w| w == b"%PDF-") {
        // PDF readers accept the header anywhere in the first KiB.
        Some(SourceFormat::Pdf)
    } else {
        None
    }
}

pub(crate) fn check_source_size(len: usize) -> Result<(), RenderError> {
    let bytes = len as u64;
    if bytes > MAX_SOURCE_BYTES {
        return Err(RenderError::SourceTooLarge { bytes, limit: MAX_SOURCE_BYTES });
    }
    Ok(())
}

pub(crate) fn check_pixels(width: u32, height: u32) -> Result<(), RenderError> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 {
        return Err(RenderError::Decode("the picture is empty".into()));
    }
    if pixels > MAX_DECODED_PIXELS {
        return Err(RenderError::TooManyPixels { pixels, limit: MAX_DECODED_PIXELS });
    }
    Ok(())
}

fn swaps_axes(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH
    )
}

fn decode_err(error: image::ImageError) -> RenderError {
    match error {
        image::ImageError::Unsupported(_) => RenderError::UnsupportedFormat,
        image::ImageError::Limits(_) => RenderError::Decode("the picture is too large to decode".into()),
        other => RenderError::Decode(other.to_string()),
    }
}

/// Decode a picture (EXIF orientation applied, transparency composited onto white) into grey.
/// `plan` sees the oriented dimensions before any pixel data is decoded.
fn decode_gray(
    bytes: &[u8],
    plan: impl FnOnce(u32, u32) -> Result<(), RenderError>,
) -> Result<image::GrayImage, RenderError> {
    check_source_size(bytes.len())?;
    let format = sniff_format(bytes).and_then(SourceFormat::image_format).ok_or(RenderError::UnsupportedFormat)?;
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format).into_decoder().map_err(decode_err)?;
    let (width, height) = decoder.dimensions();
    check_pixels(width, height)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let (oriented_w, oriented_h) = if swaps_axes(orientation) { (height, width) } else { (width, height) };
    plan(oriented_w, oriented_h)?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(1 << 30);
    decoder.set_limits(limits).map_err(decode_err)?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(decode_err)?;
    image.apply_orientation(orientation);
    Ok(composite_on_white(image))
}

/// Render a picture file (PNG, JPEG, BMP, GIF, WebP, TIFF) at the target width.
///
/// # Errors
///
/// [`RenderError::InvalidParam`] for a bad target or tone settings,
/// [`RenderError::SourceTooLarge`], [`RenderError::TooManyPixels`] and [`RenderError::TooLong`]
/// for inputs over the limits (checked before heavy work), [`RenderError::UnsupportedFormat`]
/// when the leading bytes are not a known picture, and [`RenderError::Decode`] for a corrupt file.
///
/// ```
/// use nuprint_render::{ImageParams, RenderTarget, Tone, decode_png_mono, encode_png, render_image};
/// use nuprint_protocol::MonoBitmap;
///
/// // A 384 x 8 image that is black on its left half only.
/// let source = MonoBitmap::from_fn(384, 8, |x, _| x < 192)?;
/// let png = encode_png(&source);
/// let params = ImageParams { tone: Tone::Threshold, ..ImageParams::default() };
/// let page = render_image(&png, &params, &RenderTarget::default())?;
/// assert_eq!(page.bitmap, source); // already at the target width: no resampling
/// assert_eq!(decode_png_mono(&png)?, source);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn render_image(bytes: &[u8], params: &ImageParams, target: &RenderTarget) -> Result<RenderOutput, RenderError> {
    target.validate()?;
    params.validate()?;
    let rotation = params.rotation;
    let gray = decode_gray(bytes, |width, height| {
        let (w, h) = raster::rotated_dims(width, height, rotation);
        raster::check_height(raster::scaled_height(w, h, target.width_dots), target)
    })?;
    raster::finish(gray, params, target)
}

/// 1-bit grayscale PNG with black dots as luminance 0; the preview and export format.
///
/// # Panics
///
/// Never for a valid [`MonoBitmap`]; the internal expectations only restate that writing to
/// memory cannot fail and that the packed data length matches the header.
pub fn encode_png(bitmap: &MonoBitmap) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, bitmap.width(), bitmap.height());
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::One);
    encoder.set_compression(png::Compression::Fast);
    // PNG grayscale 1 = white. Canonical padding bits are always 0, so they invert to white.
    let data: Vec<u8> = bitmap.data().iter().map(|byte| !byte).collect();
    let mut writer = encoder.write_header().expect("writing to a Vec cannot fail");
    writer.write_image_data(&data).expect("PNG data length matches the header");
    writer.finish().expect("writing to a Vec cannot fail");
    out
}

/// Read a PNG back to the canonical bitmap without resampling or dithering: luminance below
/// 128 is black. [`encode_png`] output round-trips exactly.
///
/// # Errors
///
/// [`RenderError::SourceTooLarge`] or [`RenderError::TooManyPixels`] over the limits,
/// [`RenderError::UnsupportedFormat`] unless the bytes are a PNG, and [`RenderError::Decode`]
/// for a corrupt PNG.
pub fn decode_png_mono(bytes: &[u8]) -> Result<MonoBitmap, RenderError> {
    check_source_size(bytes.len())?;
    if sniff_format(bytes) != Some(SourceFormat::Png) {
        return Err(RenderError::UnsupportedFormat);
    }
    let decoder = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png).into_decoder().map_err(decode_err)?;
    let (width, height) = decoder.dimensions();
    check_pixels(width, height)?;
    let image = DynamicImage::from_decoder(decoder).map_err(decode_err)?;
    let gray = composite_on_white(image);
    MonoBitmap::from_fn(width, height, |x, y| gray.get_pixel(x, y).0[0] < 128)
        .map_err(|e| RenderError::Decode(e.to_string()))
}
