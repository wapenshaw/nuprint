//! PDF page rendering through a dynamically bound PDFium.
//!
//! PDFium is not re-entrant and pdfium-render binds it once per process, so the engine is a
//! process-wide singleton behind a mutex; every document, page and bitmap lives and dies
//! inside one lock hold.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use image::{DynamicImage, GrayImage};
use pdfium_render::prelude::*;

use crate::codec::{check_pixels, check_source_size};
use crate::raster::{self, composite_on_white};
use crate::{ImageParams, MAX_DECODED_PIXELS, RenderError, RenderOutput, RenderTarget, RenderWarning};

/// Where to look for the PDFium library, most specific first: `NUPRINT_PDFIUM_DIR`, the
/// caller's bundled-resource directories, the executable's directory and, in debug builds, the
/// workspace `vendor/pdfium/<platform>` tree that `scripts/fetch-pdfium.*` populates.
pub fn pdfium_search_dirs(resource_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("NUPRINT_PDFIUM_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    dirs.extend(resource_dirs.iter().cloned());
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)) {
        dirs.push(exe_dir);
    }
    if cfg!(debug_assertions) {
        let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "aarch64") => "win-arm64",
            ("windows", _) => "win-x64",
            ("macos", "aarch64") => "mac-arm64",
            ("macos", _) => "mac-x64",
            (_, "aarch64") => "linux-arm64",
            _ => "linux-x64",
        };
        let vendor = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/pdfium").join(platform);
        dirs.push(vendor.join("bin"));
        dirs.push(vendor.join("lib"));
    }
    dirs
}

/// Render at this multiple of the target width, then downsample (Lanczos3) for crisp text.
const SUPERSAMPLE: u32 = 2;

type Shared = Arc<Mutex<Pdfium>>;

static LOADED: Mutex<Option<Shared>> = Mutex::new(None);

/// Cheap, cloneable handle to the process-wide PDFium instance. `Send + Sync`; all PDFium access
/// is serialized.
#[derive(Clone)]
pub struct PdfEngine {
    pdfium: Shared,
}

impl std::fmt::Debug for PdfEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PdfEngine")
    }
}

impl PdfEngine {
    /// Bind PDFium from the first of `search_dirs` that holds the platform library, then from
    /// the system library path. Later calls return the already loaded engine.
    ///
    /// # Errors
    ///
    /// [`RenderError::PdfUnavailable`] when no PDFium library can be loaded.
    ///
    /// The example only compiles; running it needs PDFium. The behavior is covered by the
    /// integration suite in `tests/pdf.rs` (set `NUPRINT_REQUIRE_PDFIUM=1` to make a missing
    /// library fail rather than skip).
    ///
    /// ```no_run
    /// use nuprint_render::{ImageParams, PdfEngine, RenderTarget, pdfium_search_dirs};
    ///
    /// # fn main() -> Result<(), nuprint_render::RenderError> {
    /// let engine = PdfEngine::load(&pdfium_search_dirs(&[]))?;
    /// let pdf = std::fs::read("document.pdf").expect("a PDF to render");
    /// let pages = engine.page_count(&pdf, None)?;
    /// let first = engine.render_page(&pdf, None, 1, &ImageParams::default(), &RenderTarget::default())?;
    /// assert!(pages >= 1 && first.bitmap.width() == 384);
    /// # Ok(())
    /// # }
    /// ```
    pub fn load(search_dirs: &[PathBuf]) -> Result<Self, RenderError> {
        let mut loaded = LOADED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = loaded.as_ref() {
            return Ok(Self { pdfium: Arc::clone(existing) });
        }
        let mut tried = Vec::new();
        for dir in search_dirs {
            let path = Pdfium::pdfium_platform_library_name_at_path(dir);
            match Pdfium::bind_to_library(&path) {
                Ok(bindings) => return Ok(Self::install(&mut loaded, bindings)),
                Err(error) => tried.push(format!("{}: {error}", path.display())),
            }
        }
        match Pdfium::bind_to_system_library() {
            Ok(bindings) => Ok(Self::install(&mut loaded, bindings)),
            Err(error) => {
                tried.push(format!("system library path: {error}"));
                Err(RenderError::PdfUnavailable(format!(
                    "the PDFium library was not found (tried {})",
                    tried.join("; ")
                )))
            }
        }
    }

    fn install(slot: &mut Option<Shared>, bindings: Box<dyn PdfiumLibraryBindings>) -> Self {
        let shared: Shared = Arc::new(Mutex::new(Pdfium::new(bindings)));
        *slot = Some(Arc::clone(&shared));
        Self { pdfium: shared }
    }

    fn lock(&self) -> MutexGuard<'_, Pdfium> {
        self.pdfium.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Number of pages in the document.
    ///
    /// # Errors
    ///
    /// [`RenderError::SourceTooLarge`], [`RenderError::PdfPasswordRequired`] when the document
    /// is encrypted and the password is missing or wrong, and [`RenderError::Pdf`] when it
    /// cannot be parsed.
    pub fn page_count(&self, bytes: &[u8], password: Option<&str>) -> Result<u32, RenderError> {
        check_source_size(bytes.len())?;
        let pdfium = self.lock();
        let document = open(&pdfium, bytes, password)?;
        Ok(document.pages().len() as u32)
    }

    /// Render one page (1-based) at the target width.
    ///
    /// # Errors
    ///
    /// Those of [`Self::page_count`], plus [`RenderError::InvalidParam`] for a bad target or
    /// tone settings, [`RenderError::PdfPageOutOfRange`] and [`RenderError::TooLong`].
    pub fn render_page(
        &self,
        bytes: &[u8],
        password: Option<&str>,
        page_number: u32,
        params: &ImageParams,
        target: &RenderTarget,
    ) -> Result<RenderOutput, RenderError> {
        target.validate()?;
        params.validate()?;
        check_source_size(bytes.len())?;
        let (gray, view_w) = self.rasterize_page(bytes, password, page_number, params, target)?;
        // PDFium is released here; post-processing doesn't need the lock.
        let mut output = raster::finish(gray, params, target)?;

        // Informational: physical page width versus the printable width.
        let page_mm = f64::from(view_w) * 25.4 / 72.0;
        let printed_mm = f64::from(target.width_dots) / f64::from(target.dots_per_mm);
        let factor = (printed_mm / page_mm) as f32;
        if factor < 0.999 {
            output.warnings.insert(0, RenderWarning::ContentScaledDown { factor });
        }
        Ok(output)
    }

    /// Render one page to grayscale at the supersampled width, under the PDFium lock. Also
    /// returns the rotated page width in points.
    fn rasterize_page(
        &self,
        bytes: &[u8],
        password: Option<&str>,
        page_number: u32,
        params: &ImageParams,
        target: &RenderTarget,
    ) -> Result<(GrayImage, f32), RenderError> {
        let pdfium = self.lock();
        let document = open(&pdfium, bytes, password)?;
        let count = document.pages().len() as u32;
        if page_number == 0 || page_number > count {
            return Err(RenderError::PdfPageOutOfRange { page: page_number, count });
        }
        let page = document.pages().get((page_number - 1) as PdfPageIndex).map_err(pdf_err)?;
        let (points_w, points_h) = (page.width().value, page.height().value);
        if !(points_w.is_finite() && points_h.is_finite() && points_w > 0.0 && points_h > 0.0) {
            return Err(RenderError::Pdf("the page has no usable size".into()));
        }

        // Reject before rendering: rotated page height at the target width.
        let (view_w, view_h) = match params.rotation {
            crate::Rotation::R0 | crate::Rotation::R180 => (points_w, points_h),
            crate::Rotation::R90 | crate::Rotation::R270 => (points_h, points_w),
        };
        let rows = (f64::from(view_h) * f64::from(target.width_dots) / f64::from(view_w)).round().max(1.0) as u64;
        raster::check_height(rows, target)?;

        let render_w = target.width_dots * SUPERSAMPLE;
        let render_h = (f64::from(points_h) * f64::from(render_w) / f64::from(points_w)).round().max(1.0);
        if render_h > f64::from(i32::MAX) || u64::from(render_w) * render_h as u64 > MAX_DECODED_PIXELS {
            return Err(RenderError::TooManyPixels {
                pixels: u64::from(render_w) * render_h as u64,
                limit: MAX_DECODED_PIXELS,
            });
        }
        check_pixels(render_w, render_h as u32)?;

        let config = PdfRenderConfig::new()
            .set_fixed_size(render_w as Pixels, render_h as Pixels)
            .set_format(PdfBitmapFormat::BGRA)
            .set_clear_color(PdfColor::WHITE)
            .render_form_data(true);
        let bitmap = page.render_with_config(&config).map_err(pdf_err)?;
        let image: DynamicImage = bitmap.as_image().map_err(pdf_err)?;
        Ok((composite_on_white(&image), view_w))
    }
}

fn pdf_err(error: PdfiumError) -> RenderError {
    RenderError::Pdf(error.to_string())
}

fn open<'a>(pdfium: &'a Pdfium, bytes: &'a [u8], password: Option<&str>) -> Result<PdfDocument<'a>, RenderError> {
    pdfium.load_pdf_from_byte_slice(bytes, password).map_err(|error| match error {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => RenderError::PdfPasswordRequired,
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FormatError) => {
            RenderError::Pdf("this does not look like a valid PDF file".into())
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::SecurityError) => {
            RenderError::Pdf("this PDF uses a security setting that is not supported".into())
        }
        other => pdf_err(other),
    })
}
