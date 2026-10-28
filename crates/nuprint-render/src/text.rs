//! Text layout and rasterization with cosmic-text and the bundled Noto fonts only.

use std::collections::BTreeSet;
use std::sync::{LazyLock, Mutex, MutexGuard};

use cosmic_text::fontdb::Database;
use cosmic_text::{
    Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight, Wrap,
    fallback::Fallback,
};
use nuprint_protocol::MonoBitmap;
use unicode_script::Script;

use crate::raster::{check_height, with_blank_warning};
use crate::{
    FontFamily, MAX_TEXT_CHARS, MIN_TEXT_LINE_WIDTH, RenderError, RenderOutput, RenderTarget, RenderWarning, TextAlign,
    TextParams,
};

const SANS: &str = "Noto Sans";
const MONO: &str = "Noto Sans Mono";
const HEADING_SCALE: f32 = 1.5;
/// Glyph coverage (0..=255) from which a dot is black. Matches the Python renderer's
/// luma-160 cut: 255 - 160 = 95.
const COVERAGE_CUTOFF: u8 = 96;

static FONT_FILES: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/NotoSans-Regular.ttf"),
    include_bytes!("../assets/fonts/NotoSans-Bold.ttf"),
    include_bytes!("../assets/fonts/NotoSansMono-Regular.ttf"),
    include_bytes!("../assets/fonts/NotoSansMono-Bold.ttf"),
];

/// No script or common fallbacks: only the bundled families are ever used.
struct NoFallback;

impl Fallback for NoFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &[]
    }
    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }
    fn script_fallback(&self, _script: Script, _locale: &str) -> &[&'static str] {
        &[]
    }
}

struct TextEngine {
    fonts: FontSystem,
    swash: SwashCache,
}

static ENGINE: LazyLock<Mutex<TextEngine>> = LazyLock::new(|| {
    let mut db = Database::new();
    for data in FONT_FILES {
        db.load_font_data(data.to_vec());
    }
    db.set_sans_serif_family(SANS);
    db.set_monospace_family(MONO);
    let fonts = FontSystem::new_with_locale_and_db_and_fallback("en-US".to_string(), db, NoFallback);
    Mutex::new(TextEngine { fonts, swash: SwashCache::new() })
});

fn engine() -> MutexGuard<'static, TextEngine> {
    ENGINE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn validate(params: &TextParams, target: &RenderTarget) -> Result<(), RenderError> {
    target.validate()?;
    if params.text.trim().is_empty() {
        return Err(RenderError::EmptyText);
    }
    if params.text.chars().count() > MAX_TEXT_CHARS {
        return Err(RenderError::InvalidParam("Text is too long; split it into smaller prints".into()));
    }
    if !(12..=96).contains(&params.size_px) {
        return Err(RenderError::InvalidParam("Font size must be 12-96 pixels".into()));
    }
    if !params.line_spacing.is_finite() || !(0.8..=3.0).contains(&params.line_spacing) {
        return Err(RenderError::InvalidParam("Line spacing must be between 0.8 and 3.0".into()));
    }
    let margins = u32::from(params.margin_dots) * 2;
    if target.width_dots.saturating_sub(margins) < MIN_TEXT_LINE_WIDTH {
        return Err(RenderError::InvalidParam(format!(
            "The margins are too wide: at least {MIN_TEXT_LINE_WIDTH} dots of line width must remain"
        )));
    }
    Ok(())
}

/// A paragraph's text and style after heading/tab handling.
struct Paragraph {
    text: String,
    size: f32,
    bold: bool,
}

fn paragraphs(params: &TextParams) -> Vec<Paragraph> {
    let normalized = params.text.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ");
    normalized
        .split('\n')
        .map(|line| match line.strip_prefix("# ").filter(|_| params.headings) {
            Some(heading) => {
                Paragraph { text: heading.to_string(), size: f32::from(params.size_px) * HEADING_SCALE, bold: true }
            }
            None => Paragraph { text: line.to_string(), size: f32::from(params.size_px), bold: params.bold },
        })
        .collect()
}

fn is_invisible(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(
            c,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}'
        )
}

/// Render text into the canonical bitmap: wrapped by glyph width, thresholded without dithering.
pub fn render_text(params: &TextParams, target: &RenderTarget) -> Result<RenderOutput, RenderError> {
    validate(params, target)?;
    let family_name = match params.family {
        FontFamily::Sans => SANS,
        FontFamily::Mono => MONO,
    };
    let margin = u32::from(params.margin_dots);
    let line_width = (target.width_dots - 2 * margin) as f32;
    let align = match params.align {
        TextAlign::Left => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Right,
    };
    let paragraphs = paragraphs(params);
    let base = Attrs::new().family(Family::Name(family_name));
    let body_metrics = Metrics::new(f32::from(params.size_px), f32::from(params.size_px) * params.line_spacing);

    let mut guard = engine();
    let TextEngine { fonts, swash } = &mut *guard;

    let last = paragraphs.len() - 1;
    let pieces: Vec<(String, Attrs)> = paragraphs
        .iter()
        .enumerate()
        .map(|(index, paragraph)| {
            let mut text = paragraph.text.clone();
            if index != last {
                text.push('\n');
            }
            let weight = if paragraph.bold { Weight::BOLD } else { Weight::NORMAL };
            let metrics = Metrics::new(paragraph.size, paragraph.size * params.line_spacing);
            (text, base.clone().weight(weight).metrics(metrics))
        })
        .collect();

    let mut buffer = Buffer::new_empty(body_metrics);
    buffer.set_wrap(Wrap::WordOrGlyph);
    buffer.set_size(Some(line_width), None);
    buffer.set_rich_text(
        pieces.iter().map(|(text, attrs)| (text.as_str(), attrs.clone())),
        &base,
        Shaping::Advanced,
        Some(align),
    );
    buffer.shape_until_scroll(fonts, false);

    let mut content_height = 0.0f32;
    let mut missing = BTreeSet::new();
    for run in buffer.layout_runs() {
        content_height = content_height.max(run.line_top + run.line_height);
        for glyph in run.glyphs {
            if glyph.glyph_id == 0
                && let Some(cluster) = run.text.get(glyph.start..glyph.end)
            {
                missing.extend(cluster.chars().filter(|c| !is_invisible(*c)));
            }
        }
    }
    let height = u64::from(margin) * 2 + content_height.ceil().max(1.0) as u64;
    check_height(height, target)?;
    let (width, height) = (target.width_dots, height as u32);

    // Allocated only after the height limit passed. A dot is black when any glyph covers it
    // at least COVERAGE_CUTOFF.
    let mut bitmap = MonoBitmap::new(width, height).map_err(|e| RenderError::InvalidParam(e.to_string()))?;
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let physical = glyph.physical((margin as f32, margin as f32 + run.line_y), 1.0);
            let Some(image) = swash.get_image(fonts, physical.cache_key).as_ref() else { continue };
            let left = physical.x + image.placement.left;
            let top = physical.y - image.placement.top;
            let (glyph_w, glyph_h) = (image.placement.width as i32, image.placement.height as i32);
            for row in 0..glyph_h {
                let y = top + row;
                if y < 0 || y >= height as i32 {
                    continue;
                }
                for col in 0..glyph_w {
                    let x = left + col;
                    if x < 0 || x >= width as i32 {
                        continue;
                    }
                    let index = (row * glyph_w + col) as usize;
                    let alpha = match image.content {
                        SwashContent::Mask => image.data[index],
                        SwashContent::Color => image.data[index * 4 + 3],
                        SwashContent::SubpixelMask => continue,
                    };
                    if alpha >= COVERAGE_CUTOFF {
                        bitmap.set(x as u32, y as u32, true);
                    }
                }
            }
        }
    }

    let mut warnings = Vec::new();
    if !missing.is_empty() {
        warnings.push(RenderWarning::MissingGlyphs { chars: missing.into_iter().collect() });
    }
    Ok(with_blank_warning(bitmap, warnings))
}
