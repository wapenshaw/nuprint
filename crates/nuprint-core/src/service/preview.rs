//! Opened files, preview rendering and the preview snapshots that jobs are admitted from.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use nuprint_protocol::MonoBitmap;
use nuprint_render::{ImageParams, RenderError, RenderOutput, RenderTarget, RenderWarning, SourceFormat, TextParams};
use uuid::Uuid;

use crate::api::*;
use crate::error::{AppError, AppResult, ErrorCode};

use super::{AppService, blocking_io};

pub(super) struct Source {
    format: SourceFormat,
    bytes: Arc<Vec<u8>>,
}

pub(super) struct Snapshot {
    pub(super) profile_id: String,
    pub(super) profile_revision: u32,
    pub(super) source_kind: &'static str,
    pub(super) settings_json: String,
    /// (1-based source page, canonical bitmap)
    pub(super) pages: Vec<(u32, MonoBitmap)>,
    png: Arc<Vec<u8>>,
}

/// A finished preview render: the bitmap and everything derived from it.
struct PreparedPreview {
    source_page: u32,
    bitmap: MonoBitmap,
    png: Arc<Vec<u8>>,
    sha256: String,
    black_ratio: f32,
    warnings: Vec<String>,
}

/// Read a user-chosen file, never more than the accepted source size, even if the file grew
/// after its size was checked.
fn read_source(path: &Path) -> AppResult<Vec<u8>> {
    let unreadable = |e: std::io::Error| AppError::invalid("That file could not be opened.").with_detail(e.to_string());
    let meta = fs::metadata(path).map_err(unreadable)?;
    if !meta.is_file() {
        return Err(AppError::invalid("Choose a file, not a folder."));
    }
    let limit = nuprint_render::MAX_SOURCE_BYTES;
    if meta.len() > limit {
        return Err(RenderError::SourceTooLarge { bytes: meta.len(), limit }.into());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    fs::File::open(path).map_err(unreadable)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(RenderError::SourceTooLarge { bytes: bytes.len() as u64, limit }.into());
    }
    Ok(bytes)
}

/// The settings stored with a job: how it was made, without what it says. The text itself and a
/// PDF's password are never written to the database; a retry reprints the saved raster pages.
fn persisted_settings(spec: &SourceSpec) -> AppResult<String> {
    let mut value = serde_json::to_value(spec).map_err(AppError::internal)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("text");
        object.remove("password");
    }
    serde_json::to_string(&value).map_err(AppError::internal)
}

fn source_kind_name(spec: &SourceSpec) -> &'static str {
    match spec {
        SourceSpec::Text(_) => "text",
        SourceSpec::Image { .. } => "image",
        SourceSpec::Pdf { .. } => "pdf",
        SourceSpec::TestPattern => "test_pattern",
    }
}

fn image_params(adjust: &ImageAdjust) -> ImageParams {
    ImageParams {
        tone: match adjust.tone {
            Tone::Dither => nuprint_render::Tone::Dither,
            Tone::Atkinson => nuprint_render::Tone::Atkinson,
            Tone::Threshold => nuprint_render::Tone::Threshold,
        },
        rotation: match adjust.rotation {
            Rotation::R0 => nuprint_render::Rotation::R0,
            Rotation::R90 => nuprint_render::Rotation::R90,
            Rotation::R180 => nuprint_render::Rotation::R180,
            Rotation::R270 => nuprint_render::Rotation::R270,
        },
        brightness: adjust.brightness,
        contrast: adjust.contrast,
    }
}

fn text_params(spec: &TextSpec) -> TextParams {
    TextParams {
        text: spec.text.clone(),
        size_px: spec.size_px,
        family: match spec.family {
            FontFamily::Sans => nuprint_render::FontFamily::Sans,
            FontFamily::Mono => nuprint_render::FontFamily::Mono,
        },
        bold: spec.bold,
        align: match spec.align {
            TextAlign::Left => nuprint_render::TextAlign::Left,
            TextAlign::Center => nuprint_render::TextAlign::Center,
            TextAlign::Right => nuprint_render::TextAlign::Right,
        },
        line_spacing: spec.line_spacing,
        margin_dots: spec.margin_dots,
        headings: spec.headings,
    }
}

fn warning_text(warning: &RenderWarning) -> String {
    match warning {
        RenderWarning::MissingGlyphs { chars } => {
            let list: String = chars.iter().take(12).collect();
            format!("The bundled fonts cannot print these characters; they will appear as boxes: {list}")
        }
        RenderWarning::BlankOutput => "Nothing would print: the preview has no black dots.".into(),
        RenderWarning::ContentScaledDown { factor } => {
            format!(
                "The page was scaled to {:.0}% to fit the paper width; small text may be unreadable.",
                factor * 100.0
            )
        }
    }
}

impl AppService {
    /// Copy a user-selected file into app-managed memory. Only the file name is kept.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidInput`] when the file cannot be opened or is a folder.
    /// [`ErrorCode::RenderInvalid`] when it is over the size limit or not a recognised picture or
    /// PDF, or a PDF is damaged. [`ErrorCode::PdfUnavailable`] when PDFium cannot be loaded.
    /// [`ErrorCode::Cancelled`] when the service is shutting down. A PDF that needs a password is
    /// not an error; its page count is left empty.
    pub async fn import_source(&self, path: &Path) -> AppResult<SourceInfo> {
        let owned = path.to_path_buf();
        let bytes = Arc::new(blocking_io(move || read_source(&owned)).await?);
        let format = nuprint_render::sniff_format(&bytes).ok_or(RenderError::UnsupportedFormat)?;
        let name = path.file_name().map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
        let (kind, page_count) = if format == SourceFormat::Pdf {
            let service = self.clone();
            let data = bytes.clone();
            let count = self
                .inner
                .renders
                .run_exclusive(&self.inner.shutdown, move || {
                    let engine = service.pdf_engine()?;
                    Ok(engine.page_count(&data, None))
                })
                .await?;
            match count {
                Ok(count) => (SourceKind::Pdf, Some(count)),
                Err(RenderError::PdfPasswordRequired) => (SourceKind::Pdf, None),
                Err(e) => return Err(e.into()),
            }
        } else {
            (SourceKind::Image, None)
        };
        let info =
            SourceInfo { source_id: Uuid::new_v4().to_string(), kind, name, bytes: bytes.len() as f64, page_count };
        self.inner.sources.insert(info.source_id.clone(), Source { format, bytes });
        Ok(info)
    }

    fn source(&self, source_id: &str) -> AppResult<Arc<Source>> {
        self.inner
            .sources
            .get(source_id)
            .ok_or_else(|| AppError::not_found("The opened file").with_detail("open it again"))
    }

    /// Render a source to the printer's canonical bitmap and keep it as a snapshot a job can be
    /// admitted from. A request with an older `generation` than one already seen is refused.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::StalePreview`] when a newer request has replaced this one,
    /// [`ErrorCode::Cancelled`] on shutdown, [`ErrorCode::NotFound`] for an unknown profile or opened
    /// file, and [`ErrorCode::InvalidInput`] when the file's kind does not suit the source (a PDF
    /// used as a picture, or the reverse). Rendering failures arrive as
    /// [`ErrorCode::RenderInvalid`], [`ErrorCode::PdfUnavailable`] or
    /// [`ErrorCode::PdfPasswordRequired`].
    pub async fn render_preview(&self, request: PreviewRequest) -> AppResult<PreviewResult> {
        let mut ticket = self.inner.renders.admit(request.generation)?;
        let resolved =
            self.inner.registry.get(&request.profile_id).ok_or_else(|| AppError::not_found("Printer profile"))?;
        let raster = &resolved.profile.raster;
        let target = RenderTarget {
            width_dots: raster.width_dots,
            dots_per_mm: raster.dots_per_mm,
            max_height: raster.max_height,
        };
        let source_kind = source_kind_name(&request.source);
        let settings_json = persisted_settings(&request.source)?;
        // Cheap validation stays here; everything heavy runs in the bounded worker below.
        let work: Box<dyn FnOnce() -> AppResult<(u32, RenderOutput)> + Send> = match request.source.clone() {
            SourceSpec::Text(text) => {
                let params = text_params(&text);
                Box::new(move || Ok((1, nuprint_render::render_text(&params, &target)?)))
            }
            SourceSpec::TestPattern => Box::new(move || Ok((1, nuprint_render::test_pattern(&target)))),
            SourceSpec::Image { source_id, adjust } => {
                let source = self.source(&source_id)?;
                if source.format == SourceFormat::Pdf {
                    return Err(AppError::invalid("This file is a PDF; choose a PDF page instead."));
                }
                let params = image_params(&adjust);
                Box::new(move || Ok((1, nuprint_render::render_image(&source.bytes, &params, &target)?)))
            }
            SourceSpec::Pdf { source_id, page, adjust, password } => {
                let source = self.source(&source_id)?;
                if source.format != SourceFormat::Pdf {
                    return Err(AppError::invalid("This file is not a PDF."));
                }
                let service = self.clone();
                let params = image_params(&adjust);
                Box::new(move || {
                    let engine = service.pdf_engine()?;
                    let output = engine.render_page(&source.bytes, password.as_deref(), page, &params, &target)?;
                    Ok((page, output))
                })
            }
        };
        // Render, encode and measure in one bounded operation that owns the render slot.
        let prepared = self
            .inner
            .renders
            .run(&mut ticket, &self.inner.shutdown, move |gate| {
                let (source_page, output) = work()?;
                if !gate.is_current() {
                    return Ok(None);
                }
                let RenderOutput { bitmap, warnings } = output;
                let png = Arc::new(nuprint_render::encode_png(&bitmap));
                let total = f64::from(bitmap.width()) * f64::from(bitmap.height());
                Ok(Some(PreparedPreview {
                    source_page,
                    png,
                    sha256: bitmap.sha256_hex(),
                    black_ratio: (bitmap.black_pixels() as f64 / total) as f32,
                    warnings: warnings.iter().map(warning_text).collect(),
                    bitmap,
                }))
            })
            .await?;
        let snapshot_id = Uuid::new_v4().to_string();
        let result = PreviewResult {
            generation: request.generation,
            snapshot_id: snapshot_id.clone(),
            profile_id: resolved.profile.id.clone(),
            profile_revision: resolved.profile.revision,
            width: prepared.bitmap.width(),
            height: prepared.bitmap.height(),
            length_mm: prepared.bitmap.height() as f32 / target.dots_per_mm,
            sha256: prepared.sha256,
            black_ratio: prepared.black_ratio,
            warnings: prepared.warnings,
            preview_path: format!("{snapshot_id}.png"),
        };
        let snapshot = Snapshot {
            profile_id: resolved.profile.id.clone(),
            profile_revision: resolved.profile.revision,
            source_kind,
            settings_json,
            pages: vec![(prepared.source_page, prepared.bitmap)],
            png: prepared.png,
        };
        // Checked and published under one lock so a newer request is never overtaken.
        self.inner.renders.publish(&ticket, || self.inner.snapshots.insert(snapshot_id, snapshot))?;
        Ok(result)
    }

    pub(super) fn snapshot(&self, snapshot_id: &str) -> AppResult<Arc<Snapshot>> {
        self.inner
            .snapshots
            .get(snapshot_id)
            .ok_or_else(|| AppError::new(ErrorCode::StalePreview, "This preview has expired. Wait for it to refresh."))
    }

    /// Check that a preview snapshot is still available to print.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::StalePreview`] once the snapshot has been replaced or evicted, or never existed.
    pub fn check_preview(&self, snapshot_id: &str) -> AppResult<()> {
        self.snapshot(snapshot_id).map(drop)
    }

    /// PNG of the canonical bitmap: exactly what would be transmitted.
    pub fn preview_png(&self, snapshot_id: &str) -> Option<Arc<Vec<u8>>> {
        self.snapshot(snapshot_id).ok().map(|s| s.png.clone())
    }

    /// A job's first page as a PNG, for the queue: the saved page while the job keeps its print
    /// data, otherwise the preview kept when it finished. `None` for an unknown job or one
    /// whose data was removed without a preview.
    pub fn job_page_png(&self, job_id: &str) -> Option<Vec<u8>> {
        let bitmap = {
            let store = self.store();
            let job = store.get_job(job_id).ok()??;
            if !job.content_retained {
                return store.thumbnail(job_id);
            }
            let page = store.pages(job_id).ok()?.into_iter().next()?;
            store.load_page(&page).ok()?
        };
        Some(nuprint_render::encode_png(&bitmap))
    }

    /// Write a preview snapshot's PNG to `path`.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::StalePreview`] as for [`Self::check_preview`], or an I/O error writing `path`
    /// ([`ErrorCode::StorageFull`] on a full disk, otherwise [`ErrorCode::Internal`]).
    pub async fn save_preview_png(&self, snapshot_id: &str, path: &Path) -> AppResult<()> {
        let png = self.snapshot(snapshot_id)?.png.clone();
        let path = path.to_path_buf();
        blocking_io(move || Ok(fs::write(path, png.as_slice())?)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_job_settings_leave_out_passwords_and_typed_text() {
        let adjust = ImageAdjust { tone: Tone::Dither, rotation: Rotation::R0, brightness: 0, contrast: 0 };
        let pdf = SourceSpec::Pdf { source_id: "s".into(), page: 2, adjust, password: Some("hunter2".into()) };
        let saved = persisted_settings(&pdf).unwrap();
        assert!(!saved.contains("hunter2") && !saved.contains("password"), "{saved}");
        assert!(saved.contains(r#""page":2"#), "{saved}");

        let text = SourceSpec::Text(TextSpec {
            text: "private note".into(),
            size_px: 24,
            family: FontFamily::Sans,
            bold: false,
            align: TextAlign::Left,
            line_spacing: 1.0,
            margin_dots: 8,
            headings: false,
        });
        let saved = persisted_settings(&text).unwrap();
        assert!(!saved.contains("private note"), "{saved}");
        assert!(saved.contains("size_px"), "{saved}");
    }

    #[test]
    fn a_source_is_read_whole_and_never_beyond_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_source(dir.path()).unwrap_err().message, "Choose a file, not a folder.");

        let small = dir.path().join("small.png");
        fs::write(&small, b"0123456789").unwrap();
        assert_eq!(read_source(&small).unwrap(), b"0123456789");

        // Sparse, so this costs no disk space; refused from its size before anything is read.
        let large = dir.path().join("large.png");
        fs::File::create(&large).unwrap().set_len(nuprint_render::MAX_SOURCE_BYTES + 1).unwrap();
        let err = read_source(&large).unwrap_err();
        assert_eq!(err.code, ErrorCode::RenderInvalid, "{err:?}");
    }
}
