//! The source and its preview: what the controls ask to render, the debounced render itself, and
//! the print bar that says whether Print is available.

use std::time::Duration;

use nuprint_core::ErrorCode;
use nuprint_core::api::*;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::ctx::Ctx;
use crate::labels::*;
use crate::state::State;
use crate::view::*;
use crate::{AppWindow, Data};

pub(crate) const SIZE_PRESETS: [u16; 15] = [12, 16, 18, 20, 22, 24, 28, 32, 36, 40, 48, 56, 64, 72, 96];
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(250);

impl Ctx {
    pub(crate) fn current_source(st: &State, mode: i32) -> Option<SourceInfo> {
        match mode {
            1 => st.image_src.clone(),
            2 => st.pdf_src.clone(),
            _ => None,
        }
    }

    pub(crate) fn sync_file(&self, ui: &AppWindow) {
        let d = ui.global::<Data>();
        let st = self.state.lock().unwrap();
        let src = Self::current_source(&st, d.get_mode());
        d.set_has_file(src.is_some());
        d.set_ask_password(false);
        if let Some(src) = &src {
            d.set_file_name(src.name.clone().into());
            d.set_file_detail(
                match src.kind {
                    SourceKind::Image => format!("Image · {}", format_bytes(src.bytes)),
                    SourceKind::Pdf => format!(
                        "PDF · {} · {}",
                        plural(src.page_count.unwrap_or(1) as usize, "page", "pages"),
                        format_bytes(src.bytes)
                    ),
                }
                .into(),
            );
            d.set_pdf_pages(src.page_count.unwrap_or(1).max(1) as i32);
        }
        d.set_pdf_note(match st.app_info.as_ref() {
            Some(i) if !i.pdf_available => {
                i.pdf_error.clone().unwrap_or_else(|| "PDF support is not available.".into()).into()
            }
            _ => "".into(),
        });
    }

    fn adjust(ui: &AppWindow) -> ImageAdjust {
        let d = ui.global::<Data>();
        ImageAdjust {
            tone: match d.get_tone_index() {
                1 => Tone::Atkinson,
                2 => Tone::Threshold,
                _ => Tone::Dither,
            },
            rotation: match d.get_rotation_index() {
                1 => Rotation::R90,
                2 => Rotation::R180,
                3 => Rotation::R270,
                _ => Rotation::R0,
            },
            brightness: d.get_brightness().round() as i8,
            contrast: d.get_contrast().round() as i8,
        }
    }

    /// What to render for the current controls, or why there is nothing to show yet.
    fn spec(&self, ui: &AppWindow) -> Result<SourceSpec, String> {
        let d = ui.global::<Data>();
        let st = self.state.lock().unwrap();
        match d.get_mode() {
            0 => {
                let text = d.get_text_value().to_string();
                if text.trim().is_empty() {
                    return Err("Type some text to see the preview.".into());
                }
                Ok(SourceSpec::Text(TextSpec {
                    text,
                    size_px: SIZE_PRESETS.get(d.get_size_index().max(0) as usize).copied().unwrap_or(28),
                    family: if d.get_family_index() == 1 { FontFamily::Mono } else { FontFamily::Sans },
                    bold: d.get_bold(),
                    align: match d.get_align_index() {
                        1 => TextAlign::Center,
                        2 => TextAlign::Right,
                        _ => TextAlign::Left,
                    },
                    line_spacing: d.get_spacing(),
                    margin_dots: d.get_margin().max(0) as u16,
                    headings: d.get_headings(),
                }))
            }
            mode @ (1 | 2) => {
                let Some(src) = Self::current_source(&st, mode) else {
                    return Err(if mode == 1 {
                        "Open an image to see the preview."
                    } else {
                        "Open a PDF to see the preview."
                    }
                    .into());
                };
                let adjust = Self::adjust(ui);
                if mode == 1 {
                    Ok(SourceSpec::Image { source_id: src.source_id, adjust })
                } else {
                    let password = d.get_pdf_password().to_string();
                    Ok(SourceSpec::Pdf {
                        source_id: src.source_id,
                        page: d.get_pdf_page().max(1) as u32,
                        adjust,
                        password: (!password.is_empty()).then_some(password),
                    })
                }
            }
            _ => Ok(SourceSpec::TestPattern),
        }
    }

    fn show_message(&self, ui: &AppWindow, message: String, error: bool) {
        {
            let mut st = self.state.lock().unwrap();
            st.preview = None;
            st.preview_error = error;
        }
        let d = ui.global::<Data>();
        d.set_preview_kind(0);
        d.set_preview_message(message.into());
        d.set_preview_error(error);
        d.set_rendering(false);
        d.set_can_save(false);
        d.set_meta_strong("".into());
        d.set_meta_rest("".into());
        d.set_warnings(strings(Vec::new()));
        self.print_bar(ui);
    }

    /// Debounced: the preview follows the controls 250 ms after the last change.
    pub(crate) fn schedule_preview(&self, ui: &AppWindow, immediate: bool) {
        let spec = match self.spec(ui) {
            Ok(spec) => spec,
            Err(message) => {
                self.state.lock().unwrap().req_seq += 1;
                return self.show_message(ui, message, false);
            }
        };
        let (profile_id, seq) = {
            let mut st = self.state.lock().unwrap();
            let chosen = Self::selected_profile(&st).or_else(|| st.profiles.first().cloned());
            let Some(profile) = chosen else { return };
            st.req_seq += 1;
            (profile.id, st.req_seq)
        };
        ui.global::<Data>().set_rendering(true);
        self.print_bar(ui);
        let this = self.clone();
        self.rt.spawn(async move {
            if !immediate {
                tokio::time::sleep(PREVIEW_DEBOUNCE).await;
            }
            let generation = {
                let mut st = this.state.lock().unwrap();
                if st.req_seq != seq {
                    return;
                }
                st.generation += 1.0;
                st.generation
            };
            let result = this.service.render_preview(PreviewRequest { generation, profile_id, source: spec }).await;
            if this.state.lock().unwrap().req_seq != seq {
                return;
            }
            match result {
                Ok(result) => {
                    let Some(buffer) = this.service.preview_png(&result.snapshot_id).and_then(|png| decode_png(&png))
                    else {
                        return;
                    };
                    let dpm = {
                        let st = this.state.lock().unwrap();
                        Self::selected_profile(&st)
                            .or_else(|| st.profiles.first().cloned())
                            .map(|p| p.dots_per_mm)
                            .unwrap_or(8.0)
                    };
                    {
                        let mut st = this.state.lock().unwrap();
                        st.preview = Some(result.clone());
                        st.preview_error = false;
                    }
                    let ctx = this.clone();
                    this.ui(move |ui| {
                        let d = ui.global::<Data>();
                        d.set_preview_image(slint::Image::from_rgba8(buffer));
                        d.set_img_w(result.width as i32);
                        d.set_img_h(result.height as i32);
                        d.set_dpm(dpm);
                        d.set_meta_strong(format!("{} × {} dots", result.width, result.height).into());
                        d.set_meta_rest(
                            format!("· {} · {:.0}% ink", format_mm(result.length_mm), result.black_ratio * 100.0)
                                .into(),
                        );
                        d.set_warnings(strings(result.warnings.clone()));
                        d.set_ruler_ticks(ModelRc::new(VecModel::from(
                            (0..=result.length_mm.floor() as i32).collect::<Vec<_>>(),
                        )));
                        let teeth = (((result.width + 32) as f32 / 14.0).round() as i32).max(1);
                        d.set_tear_teeth(ModelRc::new(VecModel::from((0..teeth).collect::<Vec<_>>())));
                        d.set_preview_kind(1);
                        d.set_rendering(false);
                        d.set_can_save(true);
                        d.set_ask_password(false);
                        ctx.print_bar(ui);
                    });
                }
                Err(e) => {
                    if e.code == ErrorCode::StalePreview {
                        return;
                    }
                    let ctx = this.clone();
                    this.ui(move |ui| {
                        ui.global::<Data>().set_ask_password(e.code == ErrorCode::PdfPasswordRequired);
                        ctx.show_message(ui, e.message, true);
                    });
                }
            }
        });
    }

    /// The reason Print is off, the paper estimate and the feed in preview dots.
    pub(crate) fn print_bar(&self, ui: &AppWindow) {
        let d = ui.global::<Data>();
        let st = self.state.lock().unwrap();
        let mode = d.get_mode();
        let profile = Self::selected_profile(&st);
        let copies = d.get_copies().clamp(1, u16::MAX as i32) as u16;
        let feed = d.get_feed_mm().clamp(0, u16::MAX as i32) as u16;
        let dpm = profile.as_ref().map(|p| p.dots_per_mm).unwrap_or(8.0);
        d.set_feed_dots((f32::from(feed) * dpm).round() as i32);

        let reason = if st.printers.is_empty() || st.selected.is_none() {
            "Add a printer to print."
        } else if profile.is_none() {
            "This printer's profile is unavailable."
        } else if mode == 0 && d.get_text_value().trim().is_empty() {
            "Type some text to print."
        } else if (mode == 1 || mode == 2) && Self::current_source(&st, mode).is_none() {
            "Open a file to print."
        } else if d.get_rendering() {
            "Updating the preview…"
        } else if st.preview_error || st.preview.is_none() {
            "Fix the preview to print."
        } else {
            ""
        };
        match &st.preview {
            Some(p) => {
                let total = nuprint_core::jobs::paper_mm(p.length_mm, copies, feed);
                d.set_estimate_main(format!("About {} of paper", format_mm(total)).into());
                d.set_estimate_sub(
                    format!("{} × {} + {} extra feed", copies, format_mm(p.length_mm), format_mm(f32::from(feed)))
                        .into(),
                );
            }
            None => {
                d.set_estimate_main("Paper estimate".into());
                d.set_estimate_sub("Shown once there is a preview.".into());
            }
        }
        d.set_reason(reason.into());
        d.set_can_print(reason.is_empty() && !d.get_printing());
    }
}
