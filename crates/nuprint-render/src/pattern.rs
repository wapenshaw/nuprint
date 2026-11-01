//! Self-test pattern: text, a frame, a solid patch and dashed marks to check alignment, width
//! and darkness.

use nuprint_protocol::MonoBitmap;

use crate::raster::with_blank_warning;
use crate::text::render_text;
use crate::{RenderOutput, RenderTarget, TextAlign, TextParams};

const PATTERN_SIZE_PX: u16 = 22;
const MARK_AREA: u32 = 20;
/// Solid black patch across the middle half, 3 mm tall at 8 dots/mm, so photos from different
/// printers show darkness and streaks the same way.
const PATCH_GAP: u32 = 8;
const PATCH_HEIGHT: u32 = 24;

fn text_block(text: &str, align: TextAlign, target: &RenderTarget) -> Option<MonoBitmap> {
    let params = TextParams {
        text: text.to_string(),
        size_px: PATTERN_SIZE_PX,
        align,
        headings: false,
        ..TextParams::default()
    };
    render_text(&params, target).ok().map(|out| out.bitmap)
}

/// Print-alignment pattern for any width of at least 96 dots.
///
/// # Panics
///
/// If `target.width_dots` is so wide that the pattern exceeds the canonical bitmap's pixel
/// limit (64 Mi pixels: hundreds of thousands of dots wide). Printer widths are a few hundred.
pub fn test_pattern(target: &RenderTarget) -> RenderOutput {
    let width = target.width_dots.max(crate::MIN_TARGET_WIDTH);
    let target = RenderTarget { width_dots: width, ..*target };
    let heading = format!("ZEITEL / NUPRINT\n{width}-dot test\n0123456789");
    let top = text_block(&heading, TextAlign::Left, &target);
    let left = text_block("LEFT", TextAlign::Left, &target);
    let right = text_block("RIGHT", TextAlign::Right, &target);

    let top_h = top.as_ref().map_or(0, MonoBitmap::height);
    let row_h = left.as_ref().map_or(0, MonoBitmap::height);
    let patch_y = top_h + row_h + PATCH_GAP;
    let height = patch_y + PATCH_HEIGHT + MARK_AREA;
    let mut bitmap = MonoBitmap::new(width, height).expect("pattern dimensions are small and non-zero");

    let mut blit = |source: &Option<MonoBitmap>, y_offset: u32| {
        if let Some(source) = source {
            for y in 0..source.height() {
                for x in 0..source.width() {
                    if source.get(x, y) {
                        bitmap.set(x, y + y_offset, true);
                    }
                }
            }
        }
    };
    blit(&top, 0);
    blit(&left, top_h);
    blit(&right, top_h);

    for y in patch_y..patch_y + PATCH_HEIGHT {
        for x in width / 4..width * 3 / 4 {
            bitmap.set(x, y, true);
        }
    }

    for x in 0..width {
        bitmap.set(x, 0, true);
        bitmap.set(x, height - 1, true);
    }
    for y in 0..height {
        bitmap.set(0, y, true);
        bitmap.set(width - 1, y, true);
    }
    let mark_y = height - 9;
    let mut x = 12;
    while x + 3 < width.saturating_sub(12) {
        for dx in 0..=3 {
            for dy in 0..3 {
                bitmap.set(x + dx, mark_y - 1 + dy, true);
            }
        }
        x += 8;
    }
    with_blank_warning(bitmap, Vec::new())
}
