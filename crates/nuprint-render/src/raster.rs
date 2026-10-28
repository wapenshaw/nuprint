//! Grey-level plumbing shared by pictures and PDF pages: composite onto white, rotate, scale,
//! brightness/contrast, then reduce to the canonical bitmap.

use image::imageops::{self, FilterType};
use image::{DynamicImage, GrayImage, RgbaImage};
use nuprint_protocol::MonoBitmap;

use crate::{ImageParams, RenderError, RenderOutput, RenderTarget, RenderWarning, Rotation, Tone};

/// Pixels with a grey level at or above this are white under [`Tone::Threshold`].
pub(crate) const THRESHOLD: u8 = 160;

/// Convert a grey image (already at the target width) to the canonical bitmap.
///
/// # Panics
/// Panics if the image is empty or larger than `MonoBitmap`'s pixel cap; the render pipelines
/// bound sizes before calling this.
pub fn monochrome(gray: &GrayImage, tone: Tone) -> MonoBitmap {
    match tone {
        Tone::Threshold => threshold(gray),
        Tone::Dither => dither(gray),
        Tone::Atkinson => atkinson(gray),
    }
}

fn blank(gray: &GrayImage) -> MonoBitmap {
    MonoBitmap::new(gray.width(), gray.height()).expect("grey image is non-empty and within the bitmap cap")
}

fn threshold(gray: &GrayImage) -> MonoBitmap {
    let mut bitmap = blank(gray);
    let width = gray.width() as usize;
    for (y, row) in gray.as_raw().chunks_exact(width).enumerate() {
        for (x, &value) in row.iter().enumerate() {
            if value < THRESHOLD {
                bitmap.set(x as u32, y as u32, true);
            }
        }
    }
    bitmap
}

/// Floyd-Steinberg with integer arithmetic, left-to-right on every row: fully deterministic.
fn dither(gray: &GrayImage) -> MonoBitmap {
    let mut bitmap = blank(gray);
    let width = gray.width() as usize;
    // Error rows are padded by one cell on each side so edge pixels need no branches.
    let mut current = vec![0i32; width + 2];
    let mut next = vec![0i32; width + 2];
    for (y, row) in gray.as_raw().chunks_exact(width).enumerate() {
        for (x, &value) in row.iter().enumerate() {
            let wanted = i32::from(value) + current[x + 1];
            let white = wanted >= 128;
            let error = wanted - if white { 255 } else { 0 };
            if !white {
                bitmap.set(x as u32, y as u32, true);
            }
            let right = error * 7 / 16;
            let below_left = error * 3 / 16;
            let below = error * 5 / 16;
            current[x + 2] += right;
            next[x] += below_left;
            next[x + 1] += below;
            next[x + 2] += error - right - below_left - below;
        }
        std::mem::swap(&mut current, &mut next);
        next.fill(0);
    }
    bitmap
}

/// Atkinson dithering with integer arithmetic: each pixel passes 1/8 of its error to six
/// neighbours and drops the other quarter, so highlights and shadows clip to clean paper and
/// solid black: more contrast and cleaner paper than Floyd-Steinberg, at the cost of detail.
fn atkinson(gray: &GrayImage) -> MonoBitmap {
    let mut bitmap = blank(gray);
    let width = gray.width() as usize;
    // Three error rows (this one and the next two), padded by two cells on each side.
    let mut rows = [vec![0i32; width + 4], vec![0i32; width + 4], vec![0i32; width + 4]];
    for (y, row) in gray.as_raw().chunks_exact(width).enumerate() {
        for (x, &value) in row.iter().enumerate() {
            let wanted = i32::from(value) + rows[0][x + 2];
            let white = wanted >= 128;
            let share = (wanted - if white { 255 } else { 0 }) / 8;
            if !white {
                bitmap.set(x as u32, y as u32, true);
            }
            rows[0][x + 3] += share;
            rows[0][x + 4] += share;
            rows[1][x + 1] += share;
            rows[1][x + 2] += share;
            rows[1][x + 3] += share;
            rows[2][x + 2] += share;
        }
        rows.rotate_left(1);
        rows[2].fill(0);
    }
    bitmap
}

/// Rec. 601 luma of each pixel composited over white.
pub(crate) fn composite_on_white(image: &DynamicImage) -> GrayImage {
    let rgba: RgbaImage = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut out = Vec::with_capacity(width as usize * height as usize);
    for pixel in rgba.pixels() {
        let [r, g, b, a] = pixel.0;
        let luma = (299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b)) / 1000;
        let alpha = u32::from(a);
        out.push(((luma * alpha + 255 * (255 - alpha) + 127) / 255) as u8);
    }
    GrayImage::from_raw(width, height, out).expect("buffer length matches dimensions")
}

pub(crate) fn rotate(gray: GrayImage, rotation: Rotation) -> GrayImage {
    match rotation {
        Rotation::R0 => gray,
        Rotation::R90 => imageops::rotate90(&gray),
        Rotation::R180 => imageops::rotate180(&gray),
        Rotation::R270 => imageops::rotate270(&gray),
    }
}

pub(crate) fn rotated_dims(width: u32, height: u32, rotation: Rotation) -> (u32, u32) {
    match rotation {
        Rotation::R0 | Rotation::R180 => (width, height),
        Rotation::R90 | Rotation::R270 => (height, width),
    }
}

/// Rows needed to print `width x height` source pixels at the target width, aspect preserved.
pub(crate) fn scaled_height(width: u32, height: u32, target_width: u32) -> u64 {
    ((f64::from(height) * f64::from(target_width) / f64::from(width)).round() as u64).max(1)
}

pub(crate) fn check_height(rows: u64, target: &RenderTarget) -> Result<(), RenderError> {
    if rows > u64::from(target.max_height) {
        return Err(RenderError::TooLong { rows, max_rows: target.max_height });
    }
    Ok(())
}

fn adjust(gray: &mut GrayImage, brightness: i8, contrast: i8) {
    if brightness == 0 && contrast == 0 {
        return;
    }
    let factor = (100.0 + f32::from(contrast)) / 100.0;
    let offset = f32::from(brightness) * 255.0 / 100.0;
    let mut lut = [0u8; 256];
    for (value, slot) in lut.iter_mut().enumerate() {
        *slot = ((value as f32 - 128.0) * factor + 128.0 + offset).round().clamp(0.0, 255.0) as u8;
    }
    for pixel in gray.pixels_mut() {
        pixel.0[0] = lut[usize::from(pixel.0[0])];
    }
}

/// Shared tail of the picture and PDF pipelines: rotate, scale to the target width, adjust,
/// reduce to one bit. The height limit is enforced here too, as a safety net; callers also
/// check it earlier from the source dimensions so oversize inputs fail before heavy work.
pub(crate) fn finish(
    gray: GrayImage,
    params: &ImageParams,
    target: &RenderTarget,
) -> Result<RenderOutput, RenderError> {
    let mut gray = rotate(gray, params.rotation);
    let rows = scaled_height(gray.width(), gray.height(), target.width_dots);
    check_height(rows, target)?;
    if gray.width() != target.width_dots || u64::from(gray.height()) != rows {
        gray = imageops::resize(&gray, target.width_dots, rows as u32, FilterType::Lanczos3);
    }
    adjust(&mut gray, params.brightness, params.contrast);
    let bitmap = monochrome(&gray, params.tone);
    Ok(with_blank_warning(bitmap, Vec::new()))
}

pub(crate) fn with_blank_warning(bitmap: MonoBitmap, mut warnings: Vec<RenderWarning>) -> RenderOutput {
    if bitmap.black_pixels() == 0 {
        warnings.push(RenderWarning::BlankOutput);
    }
    RenderOutput { bitmap, warnings }
}
