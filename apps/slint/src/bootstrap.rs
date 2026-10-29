//! Startup: the data folder, the window backend, the service and the first fill of the screens.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nuprint_core::api::AppEvent;
use nuprint_core::{AppService, ServiceConfig};
use nuprint_transport::ble::BleTransport;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tokio::sync::mpsc;

use crate::callbacks::wire;
use crate::ctx::Ctx;
use crate::preview::SIZE_PRESETS;
use crate::state::{Prefs, State};
use crate::{AppWindow, Data, Theme};

const DEFAULT_TEXT: &str = "Hello from NuPrint!\n\nYour printer is ready.";

pub(crate) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    let _enter = runtime.enter();

    let (tx, mut events) = mpsc::unbounded_channel::<AppEvent>();
    let sink: Arc<dyn nuprint_core::EventSink> = Arc::new(move |event: AppEvent| {
        let _ = tx.send(event);
    });
    let data_dir = data_dir();
    let config = ServiceConfig {
        data_dir: Some(data_dir.clone()),
        pdfium_dirs: nuprint_render::pdfium_search_dirs(bundled_pdfium_dir().as_slice()),
        version: env!("CARGO_PKG_VERSION").into(),
    };
    select_backend()?;
    let ui = AppWindow::new()?;
    keep_size_across_monitors(&ui);
    let service = match AppService::start(config, Arc::new(BleTransport::new()), sink) {
        Ok(service) => service,
        Err(e) => {
            ui.global::<Data>().set_load_error(e.message.into());
            ui.run()?;
            return Ok(());
        }
    };

    let ctx = Ctx {
        service: service.clone(),
        ui: ui.as_weak(),
        state: Arc::new(Mutex::new(State { data_dir, ..State::default() })),
        rt: runtime.handle().clone(),
        refreshing: Arc::default(),
    };
    wire(&ui, &ctx);
    init(&ui, &ctx);

    // Service events arrive on any thread; fold them in on the UI thread.
    let event_ctx = ctx.clone();
    runtime.spawn(async move {
        while let Some(event) = events.recv().await {
            event_ctx.on_event(event);
        }
    });

    if let Some(path) = std::env::var_os("NUPRINT_SLINT_SNAPSHOT") {
        snapshot_after_start(&ui, PathBuf::from(path));
    }
    ui.run()?;
    runtime.block_on(service.shutdown(Duration::from_secs(12)));
    Ok(())
}

/// Skia drawing on the CPU: the same platform-rasterised text as Skia's GPU path, without a
/// Direct3D swap chain, which game overlays attach to and which flickered on an HDR,
/// high-refresh monitor. `SLINT_BACKEND` still overrides it.
fn select_backend() -> Result<(), slint::PlatformError> {
    if std::env::var_os("SLINT_BACKEND").is_some() {
        return Ok(());
    }
    slint::BackendSelector::new().backend_name("winit".into()).renderer_name("skia-software".into()).select()
}

/// Keep the window's size in logical pixels when it moves to a monitor with a different scale.
/// While the title bar is dragged, Windows can report a size measured at the old scale after
/// the new one took effect; Slint reads that as a larger window, so each crossing grew it by
/// the ratio of the scales. Remember the size from before the change and restore it.
fn keep_size_across_monitors(ui: &AppWindow) {
    use slint::winit_030::winit::event::WindowEvent;
    use slint::winit_030::{EventResult, WinitWindowAccessor};
    const SETTLE: Duration = Duration::from_millis(600);
    let kept: std::cell::Cell<Option<(slint::LogicalSize, Instant)>> = Default::default();
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |window, event| {
        match event {
            WindowEvent::ScaleFactorChanged { .. } => {
                // Slint applies the new scale after this filter, so this is still the old size.
                let size = match kept.get() {
                    Some((size, at)) if at.elapsed() < SETTLE => size,
                    _ => window.size().to_logical(window.scale_factor()),
                };
                kept.set(Some((size, Instant::now())));
            }
            WindowEvent::Resized(physical)
                if let Some((size, _)) = kept.get().filter(|(_, at)| at.elapsed() < SETTLE) =>
            {
                let want = size.to_physical(window.scale_factor());
                let off = |a: u32, b: u32| a.abs_diff(b) > 2;
                if off(physical.width, want.width) || off(physical.height, want.height) {
                    let weak = weak.clone();
                    slint::Timer::single_shot(Duration::ZERO, move || {
                        if let Some(ui) = weak.upgrade() {
                            ui.window().set_size(size);
                        }
                    });
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

/// Where the installers put PDFium (see `[package.metadata.packager]` in Cargo.toml): beside the
/// executable on Windows, in the app bundle's `Resources` on macOS and under
/// `/usr/lib/nuprint-slint` in the Debian package.
fn bundled_pdfium_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let resources = if cfg!(target_os = "macos") {
        exe_dir.join("../Resources")
    } else if cfg!(target_os = "windows") {
        exe_dir
    } else {
        exe_dir.join("../lib").join(env!("CARGO_BIN_NAME"))
    };
    Some(resources.join("pdfium"))
}

/// A separate data folder, so this app never touches the installed app's printers or jobs.
fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NUPRINT_SLINT_DATA") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share")))
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("NuPrint-Slint");
    // The folder's name from when this app was called the Slint demo; keep its printers and jobs.
    let old = base.join("NuPrint-SlintDemo");
    if !dir.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &dir);
    }
    dir
}

/// Development aid: with `NUPRINT_SLINT_SNAPSHOT=<dir>`, save screenshots of the main views to
/// that folder, then quit.
fn snapshot_after_start(ui: &AppWindow, dir: PathBuf) {
    let weak = ui.as_weak();
    let _ = std::fs::create_dir_all(&dir);
    let step = std::cell::Cell::new(0usize);
    // The views, then a dialog over the workspace, then the workspace in dark mode.
    let views = ["workspace", "queue", "printers", "appearance", "diagnostics", "dialog", "dark"];
    let timer = Box::leak(Box::new(slint::Timer::default()));
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(3000), move || {
        let Some(ui) = weak.upgrade() else { return };
        let page = step.get();
        if let Ok(buffer) = ui.window().take_snapshot() {
            let _ = image::save_buffer(
                dir.join(format!("{}-{}.png", page, views[page.min(views.len() - 1)])),
                buffer.as_bytes(),
                buffer.width(),
                buffer.height(),
                image::ColorType::Rgba8,
            );
        }
        if page + 1 >= views.len() {
            let _ = slint::quit_event_loop();
        } else {
            let data = ui.global::<Data>();
            match views[page + 1] {
                "dialog" => {
                    data.set_view("workspace".into());
                    data.set_dialog("clear".into());
                }
                "dark" => {
                    data.set_dialog("".into());
                    ui.global::<Theme>().set_mode(2);
                }
                view => data.set_view(view.into()),
            }
            step.set(page + 1);
        }
    });
}

/// The UI and monospace fonts the web UI's stacks resolve to on this platform. Chromium maps
/// `system-ui` to Segoe UI on Windows, and `ui-monospace` falls through to Cascadia Mono where
/// Windows ships it, else Consolas. Elsewhere the platform's default UI font is already right.
fn platform_fonts() -> (&'static str, &'static str) {
    if cfg!(windows) {
        let fonts = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts");
        let mono = if fonts.join("CascadiaMono.ttf").exists() { "Cascadia Mono" } else { "Consolas" };
        ("Segoe UI", mono)
    } else if cfg!(target_os = "macos") {
        ("", "Menlo")
    } else {
        ("", "monospace")
    }
}

fn init(ui: &AppWindow, ctx: &Ctx) {
    let data = ui.global::<Data>();
    let service = &ctx.service;
    let prefs: Prefs = std::fs::read(ctx.state.lock().unwrap().data_dir.join("prefs.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let theme = ui.global::<Theme>();
    theme.set_mode(prefs.mode);
    theme.set_accent_index(prefs.accent);
    let (font, mono) = platform_fonts();
    theme.set_font_family(font.into());
    theme.set_mono_family(mono.into());
    data.set_brand_icon(prefs.icon);

    data.set_text_value(DEFAULT_TEXT.into());
    data.set_size_model(ModelRc::new(VecModel::from(
        SIZE_PRESETS.iter().map(|s| SharedString::from(s.to_string())).collect::<Vec<_>>(),
    )));
    data.set_size_index(SIZE_PRESETS.iter().position(|s| *s == 28).unwrap_or(5) as i32);

    let logs = service.logs();
    {
        let mut st = ctx.state.lock().unwrap();
        st.prefs = prefs;
        st.app_info = Some(service.app_info());
        st.profiles = service.profiles();
        st.log_seq = logs.iter().map(|l| l.seq).fold(0.0, f64::max);
        st.logs = logs;
    }
    ctx.sync_file(ui);
    // Before the window shows, so the first preview already knows the printer.
    ctx.load_printers();
    ctx.load_jobs();
    ctx.refresh_adapter();
    ctx.sync_diagnostics();
    ctx.schedule_preview(ui, true);
}
