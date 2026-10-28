mod common;

use std::path::PathBuf;

use nuprint_render::{ImageParams, PdfEngine, RenderError, RenderTarget, RenderWarning, Rotation, Tone};

fn engine() -> Option<PdfEngine> {
    match PdfEngine::load(&nuprint_render::pdfium_search_dirs(&[])) {
        Ok(engine) => Some(engine),
        Err(error) => {
            println!("SKIPPED: PDFium is not available ({error}). Run scripts/fetch-pdfium.ps1 or fetch-pdfium.sh.");
            None
        }
    }
}

/// Hand-built PDF: page 1 is solid black 384x100pt, page 2 is blank white 384x200pt.
fn two_page_pdf() -> Vec<u8> {
    pdf_with_pages(&[(384, 100, "0 g 0 0 384 100 re f"), (384, 200, "")])
}

fn pdf_with_pages(pages: &[(u32, u32, &str)]) -> Vec<u8> {
    let mut objects: Vec<String> = vec![String::new(); 2 + pages.len() * 2];
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 3 + i * 2)).collect();
    objects[0] = "<< /Type /Catalog /Pages 2 0 R >>".into();
    objects[1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len());
    for (i, (w, h, content)) in pages.iter().enumerate() {
        let page_id = 3 + i * 2;
        let content_id = page_id + 1;
        objects[page_id - 1] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {content_id} 0 R /Resources << >> >>"
        );
        objects[content_id - 1] = format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1);
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
    );
    out
}

fn target() -> RenderTarget {
    RenderTarget::default()
}

fn threshold() -> ImageParams {
    ImageParams { tone: Tone::Threshold, ..ImageParams::default() }
}

#[test]
fn counts_and_renders_pages() {
    let Some(engine) = engine() else { return };
    let pdf = two_page_pdf();
    assert_eq!(engine.page_count(&pdf, None).unwrap(), 2);

    let black = engine.render_page(&pdf, None, 1, &threshold(), &target()).unwrap();
    assert_eq!((black.bitmap.width(), black.bitmap.height()), (384, 100));
    assert_eq!(black.bitmap.black_pixels(), 384 * 100);
    // A 135 mm wide page on a 48 mm print area is informational scaling.
    assert!(
        black
            .warnings
            .iter()
            .any(|w| matches!(w, RenderWarning::ContentScaledDown { factor } if (*factor - 0.3556).abs() < 0.01)),
        "{:?}",
        black.warnings
    );

    let white = engine.render_page(&pdf, None, 2, &threshold(), &target()).unwrap();
    assert_eq!((white.bitmap.width(), white.bitmap.height()), (384, 200));
    assert_eq!(white.bitmap.black_pixels(), 0);
    assert!(white.warnings.contains(&RenderWarning::BlankOutput));

    // Dither of solid black / white is identical to threshold.
    let dithered = engine.render_page(&pdf, None, 1, &ImageParams::default(), &target()).unwrap();
    assert_eq!(dithered.bitmap, black.bitmap);
}

#[test]
fn page_numbers_are_validated() {
    let Some(engine) = engine() else { return };
    let pdf = two_page_pdf();
    for page in [0u32, 3, 99] {
        let err = engine.render_page(&pdf, None, page, &threshold(), &target()).unwrap_err();
        assert!(matches!(err, RenderError::PdfPageOutOfRange { page: p, count: 2 } if p == page), "{err:?}");
    }
}

#[test]
fn rotation_and_height_limit() {
    let Some(engine) = engine() else { return };
    let pdf = two_page_pdf();
    let params = ImageParams { rotation: Rotation::R90, ..threshold() };
    let out = engine.render_page(&pdf, None, 1, &params, &target()).unwrap();
    assert_eq!((out.bitmap.width(), out.bitmap.height()), (384, 1475));

    let tall = pdf_with_pages(&[(100, 6000, "0 g 0 0 50 50 re f")]);
    let err = engine.render_page(&tall, None, 1, &threshold(), &target()).unwrap_err();
    assert!(matches!(err, RenderError::TooLong { rows: 23040, max_rows: 4000 }), "{err:?}");
}

#[test]
fn drawn_content_lands_where_expected() {
    let Some(engine) = engine() else { return };
    // Black square in the PDF's lower-left corner (origin bottom-left) of a 384x384 page.
    let pdf = pdf_with_pages(&[(384, 384, "0 g 0 0 96 96 re f")]);
    let out = engine.render_page(&pdf, None, 1, &threshold(), &target()).unwrap();
    assert_eq!((out.bitmap.width(), out.bitmap.height()), (384, 384));
    assert!(out.bitmap.get(10, 380) && !out.bitmap.get(10, 10) && !out.bitmap.get(380, 380));
    let (rows, cols) = (96u64, 96u64);
    assert!(out.bitmap.black_pixels().abs_diff(rows * cols) < 400, "{}", out.bitmap.black_pixels());
}

#[test]
fn garbage_is_a_readable_error() {
    let Some(engine) = engine() else { return };
    let err = engine.page_count(b"%PDF-1.4 this is not really a pdf", None).unwrap_err();
    assert!(matches!(err, RenderError::Pdf(_)), "{err:?}");
    assert!(engine.render_page(b"", None, 1, &threshold(), &target()).is_err());
}

#[test]
fn engine_is_shared_and_thread_safe() {
    let Some(engine) = engine() else { return };
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PdfEngine>();
    let pdf = std::sync::Arc::new(two_page_pdf());
    let handles: Vec<_> = (1..=2u32)
        .flat_map(|page| (0..3).map(move |_| page))
        .map(|page| {
            let engine = engine.clone();
            let pdf = pdf.clone();
            std::thread::spawn(move || {
                engine
                    .render_page(&pdf, None, page, &threshold(), &RenderTarget::default())
                    .unwrap()
                    .bitmap
                    .sha256_hex()
            })
        })
        .collect();
    let hashes: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(hashes[0], hashes[1]);
    assert_ne!(hashes[0], hashes[3]);
    // Loading again returns the same process-wide instance rather than failing to rebind.
    assert!(PdfEngine::load(&[]).is_ok());
}

#[test]
fn missing_library_reports_directories_tried() {
    // Only meaningful when nothing is loaded yet in this process and no system PDFium exists;
    // otherwise load() legitimately succeeds. Either way it must not panic.
    match PdfEngine::load(&[PathBuf::from("Z:/definitely/not/here")]) {
        Ok(_) => {}
        Err(RenderError::PdfUnavailable(message)) => assert!(message.contains("definitely")),
        Err(other) => panic!("unexpected error {other:?}"),
    }
}
