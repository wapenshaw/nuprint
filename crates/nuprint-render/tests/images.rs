mod common;

use common::{ink_bounds, png_bytes, workspace_root};
use image::{DynamicImage, GrayImage, ImageEncoder, Luma, Rgba, RgbaImage};
use nuprint_protocol::MonoBitmap;
use nuprint_render::{
    ImageParams, MAX_SOURCE_BYTES, RenderError, RenderTarget, RenderWarning, Rotation, Tone, decode_png_mono,
    encode_png, monochrome, render_image, sniff_format, test_pattern,
};

fn target() -> RenderTarget {
    RenderTarget::default()
}

fn threshold_params() -> ImageParams {
    ImageParams { tone: Tone::Threshold, ..ImageParams::default() }
}

#[test]
fn transparency_composites_to_white() {
    // Fully transparent black must print as paper, not ink.
    let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 0])));
    let out = render_image(&png_bytes(&image), &threshold_params(), &target()).unwrap();
    assert_eq!(out.bitmap.black_pixels(), 0);
    assert!(out.warnings.contains(&RenderWarning::BlankOutput));

    // Opaque black stays black.
    let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 255])));
    let out = render_image(&png_bytes(&image), &threshold_params(), &target()).unwrap();
    assert_eq!(out.bitmap.black_pixels(), u64::from(out.bitmap.width()) * u64::from(out.bitmap.height()));
}

#[test]
fn aspect_ratio_is_preserved() {
    let image = DynamicImage::ImageLuma8(GrayImage::from_pixel(768, 100, Luma([0])));
    let out = render_image(&png_bytes(&image), &ImageParams::default(), &target()).unwrap();
    assert_eq!((out.bitmap.width(), out.bitmap.height()), (384, 50));

    // Tiny images scale up and never collapse to zero rows.
    let image = DynamicImage::ImageLuma8(GrayImage::from_pixel(4000, 1, Luma([0])));
    let out = render_image(&png_bytes(&image), &ImageParams::default(), &target()).unwrap();
    assert_eq!((out.bitmap.width(), out.bitmap.height()), (384, 1));
}

#[test]
fn rotation_swaps_dimensions() {
    let image = DynamicImage::ImageLuma8(GrayImage::from_pixel(200, 100, Luma([0])));
    let bytes = png_bytes(&image);
    let r0 = render_image(&bytes, &threshold_params(), &target()).unwrap();
    assert_eq!(r0.bitmap.height(), 192);
    for rotation in [Rotation::R90, Rotation::R270] {
        let params = ImageParams { rotation, ..threshold_params() };
        let out = render_image(&bytes, &params, &target()).unwrap();
        assert_eq!(out.bitmap.height(), 768);
    }
    let params = ImageParams { rotation: Rotation::R180, ..threshold_params() };
    assert_eq!(render_image(&bytes, &params, &target()).unwrap().bitmap.height(), 192);
}

#[test]
fn rotation_is_clockwise() {
    // Black block in the top-left corner moves to the top-right after a clockwise quarter turn.
    let mut gray = GrayImage::from_pixel(100, 100, Luma([255]));
    for y in 0..30 {
        for x in 0..30 {
            gray.put_pixel(x, y, Luma([0]));
        }
    }
    let bytes = png_bytes(&DynamicImage::ImageLuma8(gray));
    let params = ImageParams { rotation: Rotation::R90, ..threshold_params() };
    let bitmap = render_image(&bytes, &params, &RenderTarget { width_dots: 100, ..target() }).unwrap().bitmap;
    let (x0, y0, x1, y1) = ink_bounds(&bitmap).unwrap();
    assert!(x0 >= 69 && x1 == 99 && y0 == 0 && y1 <= 30, "{:?}", (x0, y0, x1, y1));
}

fn jpeg_with_orientation(width: u32, height: u32, orientation: u8) -> Vec<u8> {
    let mut gray = GrayImage::from_pixel(width, height, Luma([200]));
    // Mark the top-left corner so orientation handling is observable.
    for y in 0..height / 4 {
        for x in 0..width / 4 {
            gray.put_pixel(x, y, Luma([0]));
        }
    }
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
        .write_image(gray.as_raw(), width, height, image::ExtendedColorType::L8)
        .unwrap();
    // APP1/Exif segment with a single Orientation tag (big-endian TIFF).
    let mut tiff = vec![b'M', b'M', 0, 0x2A, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, orientation, 0, 0];
    tiff.extend_from_slice(&[0, 0, 0, 0]);
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(&tiff);
    let mut segment = vec![0xFF, 0xE1];
    segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    segment.extend_from_slice(&payload);
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&segment);
    out.extend_from_slice(&jpeg[2..]);
    out
}

#[test]
fn exif_orientation_is_applied() {
    let upright = render_image(&jpeg_with_orientation(100, 50, 1), &threshold_params(), &target()).unwrap();
    assert_eq!(upright.bitmap.height(), 192);

    // Orientation 6: stored 100x50, displayed rotated 90 degrees clockwise as 50x100.
    let rotated = render_image(&jpeg_with_orientation(100, 50, 6), &threshold_params(), &target()).unwrap();
    assert_eq!((rotated.bitmap.width(), rotated.bitmap.height()), (384, 768));
    // The marked top-left corner lands in the top-right after the clockwise turn.
    let (x0, y0, x1, _) = ink_bounds(&rotated.bitmap).unwrap();
    assert!(x0 > 192 && x1 >= 380 && y0 <= 2, "{:?}", (x0, y0, x1));
}

#[test]
fn too_long_images_are_rejected_with_advice() {
    let image = DynamicImage::ImageLuma8(GrayImage::from_pixel(100, 5000, Luma([0])));
    let err = render_image(&png_bytes(&image), &ImageParams::default(), &target()).unwrap_err();
    assert!(matches!(err, RenderError::TooLong { rows: 19200, max_rows: 4000 }), "{err:?}");
    assert!(err.to_string().contains("Shorten or crop"));
    // Rotating a wide picture can make it too long too.
    let image = DynamicImage::ImageLuma8(GrayImage::from_pixel(5000, 100, Luma([0])));
    let params = ImageParams { rotation: Rotation::R90, ..ImageParams::default() };
    assert!(matches!(render_image(&png_bytes(&image), &params, &target()).unwrap_err(), RenderError::TooLong { .. }));
}

#[test]
fn oversize_source_is_rejected_before_decoding() {
    // Not an image at all: the size check must fire first.
    let bytes = vec![0u8; MAX_SOURCE_BYTES as usize + 1];
    let err = render_image(&bytes, &ImageParams::default(), &target()).unwrap_err();
    assert!(matches!(err, RenderError::SourceTooLarge { .. }), "{err:?}");
}

#[test]
fn decoded_pixel_limit_is_enforced_from_the_header() {
    // 10000 x 5000 = 50M pixels, 1-bit and all white: a tiny file that would decode huge.
    let (width, height) = (10_000u32, 5_000u32);
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::One);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&vec![0xFFu8; (width as usize).div_ceil(8) * height as usize]).unwrap();
        writer.finish().unwrap();
    }
    let err = render_image(&out, &ImageParams::default(), &target()).unwrap_err();
    assert!(matches!(err, RenderError::TooManyPixels { pixels: 50_000_000, .. }), "{err:?}");
}

#[test]
fn unsupported_and_corrupt_input() {
    assert!(matches!(
        render_image(b"definitely not a picture", &ImageParams::default(), &target()),
        Err(RenderError::UnsupportedFormat)
    ));
    let mut truncated = png_bytes(&DynamicImage::ImageLuma8(GrayImage::from_pixel(64, 64, Luma([9]))));
    truncated.truncate(40);
    assert!(matches!(render_image(&truncated, &ImageParams::default(), &target()), Err(RenderError::Decode(_))));
    // PDFs are routed through PdfEngine, not render_image.
    assert!(matches!(
        render_image(b"%PDF-1.4 ...", &ImageParams::default(), &target()),
        Err(RenderError::UnsupportedFormat)
    ));
}

#[test]
fn invalid_image_params_are_rejected() {
    let bytes = png_bytes(&DynamicImage::ImageLuma8(GrayImage::from_pixel(8, 8, Luma([0]))));
    let params = ImageParams { brightness: 101, ..ImageParams::default() };
    assert!(matches!(render_image(&bytes, &params, &target()), Err(RenderError::InvalidParam(_))));
    let narrow = RenderTarget { width_dots: 10, ..target() };
    assert!(matches!(render_image(&bytes, &ImageParams::default(), &narrow), Err(RenderError::InvalidParam(_))));
}

#[test]
fn sniffing_by_content() {
    assert_eq!(sniff_format(b"%PDF-1.7\n"), Some(nuprint_render::SourceFormat::Pdf));
    assert_eq!(sniff_format(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(nuprint_render::SourceFormat::Jpeg));
    assert_eq!(sniff_format(b"RIFF\0\0\0\0WEBPVP8 "), Some(nuprint_render::SourceFormat::Webp));
    assert_eq!(sniff_format(b"GIF89a"), Some(nuprint_render::SourceFormat::Gif));
    assert_eq!(sniff_format(b"hello"), None);
    assert_eq!(sniff_format(b""), None);
}

fn gradient(width: u32, height: u32) -> GrayImage {
    GrayImage::from_fn(width, height, |x, _| Luma([(x * 255 / (width - 1)) as u8]))
}

#[test]
fn threshold_cuts_at_160() {
    let mut gray = GrayImage::from_pixel(8, 1, Luma([255]));
    gray.put_pixel(0, 0, Luma([0]));
    gray.put_pixel(1, 0, Luma([159]));
    gray.put_pixel(2, 0, Luma([160]));
    let bitmap = monochrome(&gray, Tone::Threshold);
    assert!(bitmap.get(0, 0) && bitmap.get(1, 0));
    assert!(!bitmap.get(2, 0) && !bitmap.get(3, 0));
}

#[test]
fn dither_differs_from_threshold_and_is_deterministic() {
    let gray = gradient(384, 64);
    let threshold = monochrome(&gray, Tone::Threshold);
    let dither = monochrome(&gray, Tone::Dither);
    assert_ne!(threshold, dither);
    assert_eq!(dither, monochrome(&gray, Tone::Dither));
    assert_eq!(dither.sha256_hex(), monochrome(&gray, Tone::Dither).sha256_hex());

    // Threshold is a hard edge: all black left of the cut, all white right of it.
    let edge = (0..384).find(|&x| !threshold.get(x, 0)).unwrap();
    assert!((0..edge).all(|x| threshold.get(x, 10)) && (edge..384).all(|x| !threshold.get(x, 10)));
    // Dither keeps mid-tones: ink density follows darkness across the gradient.
    let density = |from: u32, to: u32| {
        let ink = (from..to).map(|x| (0..64).filter(|&y| dither.get(x, y)).count()).sum::<usize>();
        ink as f64 / f64::from((to - from) * 64)
    };
    assert!(density(0, 96) > 0.7 && density(288, 384) < 0.3 && (0.3..0.7).contains(&density(144, 240)));
}

#[test]
fn atkinson_is_deterministic_and_clips_highlights_and_shadows() {
    let gray = gradient(384, 64);
    let atkinson = monochrome(&gray, Tone::Atkinson);
    assert_eq!(atkinson, monochrome(&gray, Tone::Atkinson));
    assert_ne!(atkinson, monochrome(&gray, Tone::Dither));
    let flat = |level: u8, tone| monochrome(&GrayImage::from_pixel(128, 128, Luma([level])), tone).black_pixels();
    // A quarter of every error is dropped: mid-greys keep their density, while light greys
    // clip to clean paper and dark greys to solid ink (more contrast than Floyd-Steinberg).
    let density = |level: u8| flat(level, Tone::Atkinson) as f64 / (128.0 * 128.0);
    assert!((density(128) - 0.5).abs() < 0.03, "{}", density(128));
    assert_eq!(flat(255, Tone::Atkinson), 0);
    assert_eq!(flat(0, Tone::Atkinson), 128 * 128);
    assert_eq!(flat(230, Tone::Atkinson), 0, "light grey clips to paper");
    assert!(flat(230, Tone::Dither) > 0);
    assert_eq!(flat(30, Tone::Atkinson), 128 * 128, "dark grey clips to solid ink");
    assert!(flat(30, Tone::Dither) < 128 * 128);
    assert!(flat(192, Tone::Atkinson) < flat(192, Tone::Dither));
}

#[test]
fn flat_greys_dither_to_matching_density() {
    for (level, expected) in [(255u8, 0.0f64), (128, 0.5), (64, 0.75), (0, 1.0)] {
        let bitmap = monochrome(&GrayImage::from_pixel(128, 128, Luma([level])), Tone::Dither);
        let density = bitmap.black_pixels() as f64 / (128.0 * 128.0);
        assert!((density - expected).abs() < 0.03, "level {level}: {density}");
    }
}

#[test]
fn brightness_and_contrast_change_the_result() {
    let gray = DynamicImage::ImageLuma8(GrayImage::from_pixel(64, 64, Luma([120])));
    let bytes = png_bytes(&gray);
    let base = render_image(&bytes, &threshold_params(), &target()).unwrap().bitmap;
    assert_eq!(base.black_pixels(), u64::from(base.width()) * u64::from(base.height()));
    let brighter = ImageParams { brightness: 50, ..threshold_params() };
    assert_eq!(render_image(&bytes, &brighter, &target()).unwrap().bitmap.black_pixels(), 0);
    let darker = ImageParams { brightness: -50, ..threshold_params() };
    assert!(render_image(&bytes, &darker, &target()).unwrap().bitmap.black_pixels() > 0);
}

fn patterned(width: u32, height: u32) -> MonoBitmap {
    MonoBitmap::from_fn(width, height, |x, y| (x * 7 + y * 13 + x * y) % 5 < 2).unwrap()
}

#[test]
fn png_round_trip_is_exact() {
    for (w, h) in [(384, 40), (13, 7), (1, 1), (8, 3), (577, 5)] {
        let bitmap = patterned(w, h);
        let png = encode_png(&bitmap);
        assert_eq!(decode_png_mono(&png).unwrap(), bitmap, "{w}x{h}");
    }
    // Black dots are luminance 0 in the exported file.
    let one = MonoBitmap::from_fn(2, 1, |x, _| x == 0).unwrap();
    let decoded = image::load_from_memory(&encode_png(&one)).unwrap().to_luma8();
    assert_eq!((decoded.get_pixel(0, 0).0[0], decoded.get_pixel(1, 0).0[0]), (0, 255));
}

#[test]
fn decode_rejects_non_png() {
    assert!(matches!(decode_png_mono(b"nope"), Err(RenderError::UnsupportedFormat)));
}

#[test]
fn verified_strip_png_import_is_bit_exact() {
    let root = workspace_root().join("fixtures/sc03h");
    let vectors: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("vectors.json")).unwrap()).unwrap();
    let expected = vectors["verified_strip"]["rows_sha256"].as_str().unwrap();
    let bitmap = decode_png_mono(&std::fs::read(root.join("verified-strip.png")).unwrap()).unwrap();
    assert_eq!((bitmap.width(), bitmap.height()), (384, 164));
    assert_eq!(bitmap.rows_sha256_hex(), expected);
    // And re-encoding the preview keeps it identical.
    assert_eq!(decode_png_mono(&encode_png(&bitmap)).unwrap(), bitmap);
}

#[test]
fn test_pattern_fits_any_width() {
    for width in [96u32, 200, 384, 576] {
        let target = RenderTarget { width_dots: width, ..RenderTarget::default() };
        let out = test_pattern(&target);
        assert_eq!(out.bitmap.width(), width);
        assert!(out.bitmap.height() > 30 && out.bitmap.height() <= target.max_height);
        assert!(out.bitmap.black_pixels() > u64::from(width), "width {width}");
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        // Frame is present on all four sides.
        let (w, h) = (out.bitmap.width(), out.bitmap.height());
        assert!((0..w).all(|x| out.bitmap.get(x, 0) && out.bitmap.get(x, h - 1)));
        assert!((0..h).all(|y| out.bitmap.get(0, y) && out.bitmap.get(w - 1, y)));
    }
    let a = test_pattern(&RenderTarget::default());
    assert_eq!(a.bitmap, test_pattern(&RenderTarget::default()).bitmap);
}
