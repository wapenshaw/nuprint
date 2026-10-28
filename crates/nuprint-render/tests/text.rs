mod common;

use common::ink_bounds;
use nuprint_render::{
    FontFamily, MAX_TEXT_CHARS, RenderError, RenderTarget, RenderWarning, TextAlign, TextParams, render_text,
};

fn target() -> RenderTarget {
    RenderTarget::default()
}

fn params(text: &str) -> TextParams {
    TextParams { text: text.to_string(), ..TextParams::default() }
}

#[test]
fn renders_ink_inside_the_margins() {
    let out = render_text(&params("Hello, NuPrint"), &target()).unwrap();
    assert_eq!(out.bitmap.width(), 384);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let (x0, y0, x1, y1) = ink_bounds(&out.bitmap).unwrap();
    assert!(x0 >= 10 && x1 < 384 - 10, "{x0}..{x1}");
    // 12 dot top and bottom margins around one 24px line.
    assert!(y0 >= 12 && y1 < out.bitmap.height() - 6 && out.bitmap.height() > 24 + 24);
}

#[test]
fn long_words_wrap_by_glyph_width() {
    let single = render_text(&params("A"), &target()).unwrap();
    let out = render_text(&params(&"W".repeat(120)), &target()).unwrap();
    assert!(out.bitmap.height() > single.bitmap.height() * 3, "{}", out.bitmap.height());
    let (x0, _, x1, _) = ink_bounds(&out.bitmap).unwrap();
    assert!(x0 >= 10 && x1 <= 384 - 10, "ink {x0}..{x1} escaped the margins");
    assert!(out.bitmap.black_pixels() > 1000);
}

#[test]
fn words_wrap_at_spaces() {
    let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda";
    let one_line = render_text(&params("alpha beta"), &target()).unwrap();
    let wrapped = render_text(&params(text), &target()).unwrap();
    assert!(wrapped.bitmap.height() > one_line.bitmap.height());
    let (_, _, x1, _) = ink_bounds(&wrapped.bitmap).unwrap();
    assert!(x1 < 384 - 10);
}

#[test]
fn empty_and_invalid_input() {
    for text in ["", "   ", "\n\t \n"] {
        assert!(matches!(render_text(&params(text), &target()), Err(RenderError::EmptyText)), "{text:?}");
    }
    for size in [0u16, 11, 97, 1000] {
        let p = TextParams { size_px: size, ..params("x") };
        assert!(matches!(render_text(&p, &target()), Err(RenderError::InvalidParam(_))), "size {size}");
    }
    for spacing in [0.5f32, 3.5, f32::NAN, f32::INFINITY] {
        let p = TextParams { line_spacing: spacing, ..params("x") };
        assert!(matches!(render_text(&p, &target()), Err(RenderError::InvalidParam(_))), "spacing {spacing}");
    }
    // 384 - 2*160 = 64 dots is the minimum; 161 leaves 62.
    assert!(render_text(&TextParams { margin_dots: 160, ..params("x") }, &target()).is_ok());
    let p = TextParams { margin_dots: 161, ..params("x") };
    assert!(matches!(render_text(&p, &target()), Err(RenderError::InvalidParam(_))));
    let p = params(&"a".repeat(MAX_TEXT_CHARS + 1));
    assert!(matches!(render_text(&p, &target()), Err(RenderError::InvalidParam(_))));
    let narrow = RenderTarget { width_dots: 50, ..target() };
    assert!(matches!(render_text(&params("x"), &narrow), Err(RenderError::InvalidParam(_))));
}

#[test]
fn too_long_text_is_rejected_not_truncated() {
    let text = "line\n".repeat(400);
    let err = render_text(&params(&text), &target()).unwrap_err();
    assert!(matches!(err, RenderError::TooLong { max_rows: 4000, rows } if rows > 4000), "{err:?}");
    // The same text fits when the page limit is raised.
    let roomy = RenderTarget { max_height: 100_000, ..target() };
    assert!(render_text(&params(&text), &roomy).is_ok());
}

fn bounds_for(align: TextAlign) -> (u32, u32) {
    let p = TextParams { align, ..params("Hi there") };
    let out = render_text(&p, &target()).unwrap();
    let (x0, _, x1, _) = ink_bounds(&out.bitmap).unwrap();
    (x0, x1)
}

#[test]
fn alignment_moves_ink_horizontally() {
    let (left0, left1) = bounds_for(TextAlign::Left);
    let (center0, center1) = bounds_for(TextAlign::Center);
    let (right0, right1) = bounds_for(TextAlign::Right);
    assert!(left0 <= 16 && left1 < 120, "left {left0}..{left1}");
    let center = (center0 + center1) / 2;
    assert!((184..=200).contains(&center), "center {center0}..{center1}");
    assert!((384 - 12 - 6..384 - 6).contains(&right1) && right0 > 200, "right {right0}..{right1}");
}

#[test]
fn headings_are_bigger_and_bold() {
    let plain = render_text(&params("Title"), &target()).unwrap();
    let heading = render_text(&params("# Title"), &target()).unwrap();
    assert!(heading.bitmap.height() > plain.bitmap.height() + 6);
    let (px0, _, px1, _) = ink_bounds(&plain.bitmap).unwrap();
    let (hx0, _, hx1, _) = ink_bounds(&heading.bitmap).unwrap();
    assert!(hx1 - hx0 > (px1 - px0) * 13 / 10, "heading is not wider: {} vs {}", hx1 - hx0, px1 - px0);
    // The marker is stripped: no leading "#" glyph before the title.
    assert!(hx0 <= 16);
    // With headings off the line is literal text.
    let literal = TextParams { headings: false, ..params("# Title") };
    let literal = render_text(&literal, &target()).unwrap();
    assert_eq!(literal.bitmap.height(), plain.bitmap.height());
}

#[test]
fn bold_adds_ink_and_mono_differs() {
    let regular = render_text(&params("Weight 123"), &target()).unwrap();
    let bold = render_text(&TextParams { bold: true, ..params("Weight 123") }, &target()).unwrap();
    assert!(bold.bitmap.black_pixels() > regular.bitmap.black_pixels());
    let mono = render_text(&TextParams { family: FontFamily::Mono, ..params("Weight 123") }, &target()).unwrap();
    assert_ne!(mono.bitmap, regular.bitmap);
    // Monospace: ten identical advances. "iiiiiiiiii" and "WWWWWWWWWW" span the same width.
    let narrow = render_text(&TextParams { family: FontFamily::Mono, ..params("iiiiiiiiii") }, &target()).unwrap();
    let wide = render_text(&TextParams { family: FontFamily::Mono, ..params("WWWWWWWWWW") }, &target()).unwrap();
    let (n0, _, n1, _) = ink_bounds(&narrow.bitmap).unwrap();
    let (w0, _, w1, _) = ink_bounds(&wide.bitmap).unwrap();
    assert!((n1 - n0).abs_diff(w1 - w0) <= 12, "{} vs {}", n1 - n0, w1 - w0);
}

#[test]
fn line_spacing_and_blank_lines_take_room() {
    let tight = render_text(&TextParams { line_spacing: 0.8, ..params("a\nb\nc") }, &target()).unwrap();
    let loose = render_text(&TextParams { line_spacing: 2.0, ..params("a\nb\nc") }, &target()).unwrap();
    assert!(loose.bitmap.height() > tight.bitmap.height() + 40);
    let gap = render_text(&params("a\n\n\nb"), &target()).unwrap();
    let flat = render_text(&params("a\nb"), &target()).unwrap();
    assert!(gap.bitmap.height() > flat.bitmap.height() + 40, "{} {}", gap.bitmap.height(), flat.bitmap.height());
}

#[test]
fn tabs_are_four_spaces_and_newlines_normalize() {
    assert_eq!(
        render_text(&params("a\tb"), &target()).unwrap().bitmap,
        render_text(&params("a    b"), &target()).unwrap().bitmap
    );
    assert_eq!(
        render_text(&params("a\r\nb\rc"), &target()).unwrap().bitmap,
        render_text(&params("a\nb\nc"), &target()).unwrap().bitmap
    );
}

#[test]
fn missing_glyphs_are_reported() {
    let out = render_text(&params("Hello \u{6F22} \u{E000} world \u{6F22}"), &target()).unwrap();
    assert!(
        out.warnings.contains(&RenderWarning::MissingGlyphs { chars: vec!['\u{6F22}', '\u{E000}'] }),
        "{:?}",
        out.warnings
    );
    // Covered scripts do not warn.
    let out = render_text(&params("Привет, Ελλάδα, café, Việt Nam"), &target()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert!(out.bitmap.black_pixels() > 300);
}

#[test]
fn rendering_is_deterministic() {
    let p = TextParams {
        text: "# Notes\nThe quick brown fox jumps over the lazy dog.\n\tIndented\nWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW"
            .into(),
        align: TextAlign::Center,
        ..TextParams::default()
    };
    let first = render_text(&p, &target()).unwrap();
    for _ in 0..3 {
        assert_eq!(first.bitmap.sha256_hex(), render_text(&p, &target()).unwrap().bitmap.sha256_hex());
    }
    // And across threads sharing the cached font system.
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let p = p.clone();
            std::thread::spawn(move || render_text(&p, &RenderTarget::default()).unwrap().bitmap.sha256_hex())
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), first.bitmap.sha256_hex());
    }
}

#[test]
fn wider_targets_use_the_width() {
    let wide = RenderTarget { width_dots: 576, ..target() };
    let out = render_text(&params("Hello"), &wide).unwrap();
    assert_eq!(out.bitmap.width(), 576);
}
