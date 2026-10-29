//! Deterministic rendering of text, pictures and PDF pages into the canonical monochrome
//! bitmap ([`nuprint_protocol::MonoBitmap`]).
//!
//! Everything here is synchronous and pure: callers run it on a blocking thread. The only
//! global state is the cached bundled-font [`cosmic_text::FontSystem`] and, for PDFs, the
//! process-wide PDFium binding behind [`PdfEngine`].
//!
//! Text uses only the fonts bundled in `assets/fonts`; system fonts are never consulted so a
//! given input renders to the same bitmap on every platform. The preview PNG
//! ([`encode_png`]) is the printed bitmap, not a separate render path.
//!
//! ```
//! use nuprint_render::{RenderTarget, TextParams, decode_png_mono, encode_png, render_text};
//!
//! let params = TextParams { text: "NuPrint".into(), ..TextParams::default() };
//! let target = RenderTarget::default(); // SC03h: 384 dots wide
//!
//! let page = render_text(&params, &target)?;
//! assert_eq!(page.bitmap.width(), 384);
//! assert!(page.bitmap.black_pixels() > 0);
//! assert!(page.warnings.is_empty());
//!
//! // Rendering is deterministic: the bundled fonts give the same bits every time.
//! assert_eq!(page.bitmap, render_text(&params, &target)?.bitmap);
//!
//! // The preview PNG is the printed bitmap and reads back exactly.
//! let png = encode_png(&page.bitmap);
//! assert_eq!(&png[..4], b"\x89PNG");
//! assert_eq!(decode_png_mono(&png)?, page.bitmap);
//! # Ok::<(), nuprint_render::RenderError>(())
//! ```
//!
//! PDF pages go through [`PdfEngine`], which needs a native PDFium library. Its examples are
//! compile-checked only; the behavior is exercised by `tests/pdf.rs`, which CI runs with
//! `NUPRINT_REQUIRE_PDFIUM=1` so a missing library fails instead of skipping.

#![warn(clippy::missing_errors_doc, clippy::missing_panics_doc)]

mod codec;
mod pattern;
mod pdf;
mod raster;
mod text;

use nuprint_protocol::MonoBitmap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use codec::{SourceFormat, decode_png_mono, encode_png, render_image, sniff_format};
pub use pattern::test_pattern;
pub use pdf::{PdfEngine, pdfium_search_dirs};
pub use raster::monochrome;
pub use text::render_text;

/// Bump whenever a change can alter rendered pixels (fonts, layout, scaling, tone mapping).
pub const RENDERER_VERSION: &str = "nuprint-render/1";

/// Largest accepted source file (image or PDF).
pub const MAX_SOURCE_BYTES: u64 = 50 * 1024 * 1024;
/// Largest accepted decoded source image, in pixels.
pub const MAX_DECODED_PIXELS: u64 = 40_000_000;
/// Longest accepted text, in Unicode scalar values.
pub const MAX_TEXT_CHARS: usize = 20_000;

/// Smallest printable width the renderer supports.
pub const MIN_TARGET_WIDTH: u32 = 96;
/// Largest printable width the renderer supports.
pub const MAX_TARGET_WIDTH: u32 = 8192;
/// Text must keep at least this many dots of line width after margins.
pub const MIN_TEXT_LINE_WIDTH: u32 = 64;

/// The printable surface a bitmap is rendered for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RenderTarget {
    pub width_dots: u32,
    pub dots_per_mm: f32,
    pub max_height: u32,
}

impl Default for RenderTarget {
    /// The SC03h: 384 dots, 8 dots/mm, 4000 rows per page.
    fn default() -> Self {
        Self { width_dots: 384, dots_per_mm: 8.0, max_height: 4000 }
    }
}

impl RenderTarget {
    fn validate(&self) -> Result<(), RenderError> {
        if !(MIN_TARGET_WIDTH..=MAX_TARGET_WIDTH).contains(&self.width_dots) {
            return Err(RenderError::InvalidParam(format!(
                "The printable width must be between {MIN_TARGET_WIDTH} and {MAX_TARGET_WIDTH} dots (got {})",
                self.width_dots
            )));
        }
        if self.max_height == 0 {
            return Err(RenderError::InvalidParam("The maximum page height must be at least 1 row".into()));
        }
        if !(self.dots_per_mm.is_finite() && self.dots_per_mm > 0.0) {
            return Err(RenderError::InvalidParam("The print resolution must be a positive number".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Bundled families. `Serif` is intentionally absent: only Noto Sans and Noto Sans Mono ship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FontFamily {
    #[default]
    Sans,
    Mono,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextParams {
    pub text: String,
    /// Valid 12..=96.
    pub size_px: u16,
    pub family: FontFamily,
    pub bold: bool,
    pub align: TextAlign,
    /// Line height as a multiple of the font size; valid 0.8..=3.0.
    pub line_spacing: f32,
    /// Horizontal and top/bottom margin in dots; must leave at least 64 dots of line width.
    pub margin_dots: u16,
    /// Lines starting with `# ` render bold at 1.5x size with the marker removed.
    pub headings: bool,
}

impl Default for TextParams {
    fn default() -> Self {
        Self {
            text: String::new(),
            size_px: 24,
            family: FontFamily::Sans,
            bold: false,
            align: TextAlign::Left,
            line_spacing: 1.2,
            margin_dots: 12,
            headings: true,
        }
    }
}

/// How grey levels become black dots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// Floyd-Steinberg error diffusion (photos).
    #[default]
    Dither,
    /// Atkinson error diffusion: higher contrast than `Dither` (scans, line-heavy photos).
    Atkinson,
    /// Fixed threshold at 160/255 (text, line art).
    Threshold,
}

/// Clockwise rotation applied before scaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Rotation {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageParams {
    pub tone: Tone,
    pub rotation: Rotation,
    /// -100..=100
    pub brightness: i8,
    /// -100..=100
    pub contrast: i8,
}

impl Default for ImageParams {
    fn default() -> Self {
        Self { tone: Tone::Dither, rotation: Rotation::R0, brightness: 0, contrast: 0 }
    }
}

impl ImageParams {
    fn validate(&self) -> Result<(), RenderError> {
        if !(-100..=100).contains(&self.brightness) || !(-100..=100).contains(&self.contrast) {
            return Err(RenderError::InvalidParam("Brightness and contrast must be between -100 and 100".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderWarning {
    /// The bundled fonts have no glyph for these characters; a placeholder box is printed.
    MissingGlyphs { chars: Vec<char> },
    /// Nothing would print (no black dots).
    BlankOutput,
    /// The PDF page is physically wider than the printable width and was scaled down.
    ContentScaledDown { factor: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderOutput {
    pub bitmap: MonoBitmap,
    pub warnings: Vec<RenderWarning>,
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("Type some text first")]
    EmptyText,
    #[error("{0}")]
    InvalidParam(String),
    #[error(
        "This is too long for one print ({rows} rows; the limit is {max_rows}). Shorten or crop it to at most {max_rows} rows."
    )]
    TooLong { rows: u64, max_rows: u32 },
    #[error("This file is too large ({bytes} bytes; the limit is {limit} bytes)")]
    SourceTooLarge { bytes: u64, limit: u64 },
    #[error("This image has too many pixels ({pixels}; the limit is {limit}). Use a smaller picture or crop it first.")]
    TooManyPixels { pixels: u64, limit: u64 },
    #[error("Unsupported file type. Use a PNG, JPEG, BMP, GIF, WebP or TIFF picture, or a PDF.")]
    UnsupportedFormat,
    #[error("This file could not be read as a picture: {0}")]
    Decode(String),
    #[error("PDF support is unavailable: {0}")]
    PdfUnavailable(String),
    #[error("This PDF needs a password, or the password was wrong")]
    PdfPasswordRequired,
    #[error("Choose a PDF page between 1 and {count} (got {page})")]
    PdfPageOutOfRange { page: u32, count: u32 },
    #[error("The PDF could not be read: {0}")]
    Pdf(String),
}
