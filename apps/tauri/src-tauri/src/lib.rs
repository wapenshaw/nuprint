//! Tauri host for NuPrint. Thin adapter over `nuprint_core::AppService`:
//! typed commands, one event channel, a scoped preview-image protocol and native
//! dialogs run from Rust (file-system paths never cross the IPC boundary).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nuprint_core::api::*;
use nuprint_core::{AppError, AppResult, AppService, ErrorCode, ServiceConfig};
use nuprint_transport::ble::BleTransport;
use tauri::http::{Response, StatusCode, header};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, Window};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tokio::sync::oneshot;

pub const EVENT_CHANNEL: &str = "nuprint://event";

/// How many NuPrint windows can run at once, each with its own data folder.
const MAX_WINDOWS: u32 = 8;

const PREVIEW_SCHEME: &str = "nupreview";

// ---------------------------------------------------------------- dialogs

async fn pick_open(window: &Window) -> AppResult<Option<PathBuf>> {
    let (tx, rx) = oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Open a picture or PDF")
        .add_filter("Pictures and PDFs", &["png", "jpg", "jpeg", "bmp", "gif", "webp", "tif", "tiff", "pdf"])
        .add_filter("Pictures", &["png", "jpg", "jpeg", "bmp", "gif", "webp", "tif", "tiff"])
        .add_filter("PDF", &["pdf"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    to_path(rx.await.ok().flatten())
}

async fn pick_save(
    window: &Window,
    title: &str,
    file_name: &str,
    filter: &str,
    ext: &str,
) -> AppResult<Option<PathBuf>> {
    let (tx, rx) = oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title(title)
        .set_file_name(file_name)
        .add_filter(filter, &[ext])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    to_path(rx.await.ok().flatten())
}

fn to_path(path: Option<FilePath>) -> AppResult<Option<PathBuf>> {
    path.map(|p| {
        p.into_path().map_err(|e| AppError::invalid("That location cannot be used.").with_detail(e.to_string()))
    })
    .transpose()
}

// ---------------------------------------------------------------- commands

#[tauri::command]
async fn get_app_info(svc: State<'_, AppService>) -> AppResult<AppInfo> {
    Ok(svc.app_info())
}

#[tauri::command]
async fn list_profiles(svc: State<'_, AppService>) -> AppResult<Vec<ProfileSummary>> {
    Ok(svc.profiles())
}

#[tauri::command]
async fn adapter_status(svc: State<'_, AppService>) -> AppResult<AdapterStatus> {
    Ok(svc.adapter_status().await)
}

#[tauri::command]
async fn start_scan(svc: State<'_, AppService>, request: StartScanRequest) -> AppResult<ScanStarted> {
    svc.start_scan(request).await
}

#[tauri::command]
async fn stop_scan(svc: State<'_, AppService>, scan_id: String) -> AppResult<()> {
    svc.stop_scan(&scan_id);
    Ok(())
}

#[tauri::command]
async fn inspect_device(svc: State<'_, AppService>, discovery_id: String) -> AppResult<DeviceInspection> {
    svc.inspect_device(&discovery_id).await
}

#[tauri::command]
async fn cancel_inspect(svc: State<'_, AppService>) -> AppResult<()> {
    svc.cancel_inspections();
    Ok(())
}

/// Open the macOS Bluetooth privacy settings, where NuPrint's access is granted. It never
/// changes the setting itself; other systems have no per-app Bluetooth permission.
#[tauri::command]
async fn open_bluetooth_settings() -> AppResult<()> {
    if cfg!(target_os = "macos") {
        std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth")
            .status()
            .map_err(AppError::internal)?;
        Ok(())
    } else {
        Err(AppError::new(ErrorCode::InvalidInput, "Bluetooth permission is only set per app on macOS."))
    }
}

#[tauri::command]
async fn list_printers(svc: State<'_, AppService>) -> AppResult<Vec<SavedPrinter>> {
    svc.list_printers()
}

#[tauri::command]
async fn save_printer(svc: State<'_, AppService>, request: SavePrinterRequest) -> AppResult<SavedPrinter> {
    svc.save_printer(request)
}

#[tauri::command]
async fn get_printer_profile(svc: State<'_, AppService>, printer_id: String) -> AppResult<PrinterProfileDoc> {
    svc.printer_profile_doc(&printer_id)
}

#[tauri::command]
async fn save_printer_profile(
    svc: State<'_, AppService>,
    request: SavePrinterProfileRequest,
) -> AppResult<SavedPrinter> {
    svc.save_printer_profile(request)
}

#[tauri::command]
async fn reset_printer_profile(svc: State<'_, AppService>, printer_id: String) -> AppResult<SavedPrinter> {
    svc.reset_printer_profile(&printer_id)
}

#[tauri::command]
async fn export_printer_profile(window: Window, svc: State<'_, AppService>, printer_id: String) -> AppResult<bool> {
    let doc = svc.printer_profile_doc(&printer_id)?;
    let name = format!("{}-custom.json", doc.base_profile_id);
    let Some(path) = pick_save(&window, "Export printer profile", &name, "Printer profile", "json").await? else {
        return Ok(false);
    };
    svc.save_printer_profile_file(&printer_id, &path).await.map(|()| true)
}

#[tauri::command]
async fn remove_printer(svc: State<'_, AppService>, printer_id: String) -> AppResult<()> {
    svc.remove_printer(&printer_id)
}

#[tauri::command]
async fn update_printer_settings(
    svc: State<'_, AppService>,
    printer_id: String,
    settings: PrinterSettings,
) -> AppResult<SavedPrinter> {
    svc.update_printer_settings(&printer_id, settings)
}

#[tauri::command]
async fn get_selected_printer(svc: State<'_, AppService>) -> AppResult<Option<String>> {
    svc.selected_printer()
}

#[tauri::command]
async fn select_printer(svc: State<'_, AppService>, printer_id: Option<String>) -> AppResult<()> {
    svc.select_printer(printer_id)
}

#[tauri::command]
async fn check_printer(svc: State<'_, AppService>, printer_id: String) -> AppResult<PrinterCheck> {
    svc.check_printer(&printer_id).await
}

#[tauri::command]
async fn open_source(window: Window, svc: State<'_, AppService>) -> AppResult<Option<SourceInfo>> {
    let Some(path) = pick_open(&window).await? else { return Ok(None) };
    svc.import_source(&path).await.map(Some)
}

#[tauri::command]
async fn render_preview(svc: State<'_, AppService>, request: PreviewRequest) -> AppResult<PreviewResult> {
    svc.render_preview(request).await
}

#[tauri::command]
async fn save_preview_png(window: Window, svc: State<'_, AppService>, snapshot_id: String) -> AppResult<bool> {
    // Don't ask for a location for a preview that can no longer be saved.
    svc.check_preview(&snapshot_id)?;
    let Some(path) = pick_save(&window, "Save preview", "nuprint-preview.png", "PNG image", "png").await? else {
        return Ok(false);
    };
    svc.save_preview_png(&snapshot_id, &path).await.map(|()| true)
}

#[tauri::command]
async fn enqueue_job(svc: State<'_, AppService>, request: EnqueueRequest) -> AppResult<JobSummary> {
    svc.enqueue(request)
}

#[tauri::command]
async fn list_jobs(svc: State<'_, AppService>) -> AppResult<QueueSnapshot> {
    svc.list_jobs()
}

#[tauri::command]
async fn get_job(svc: State<'_, AppService>, job_id: String) -> AppResult<JobSummary> {
    svc.get_job(&job_id)
}

#[tauri::command]
async fn cancel_job(svc: State<'_, AppService>, job_id: String) -> AppResult<()> {
    svc.cancel_job(&job_id)
}

#[tauri::command]
async fn retry_job(svc: State<'_, AppService>, job_id: String, idempotency_key: String) -> AppResult<JobSummary> {
    svc.retry_job(&job_id, idempotency_key)
}

#[tauri::command]
async fn remove_job(svc: State<'_, AppService>, job_id: String) -> AppResult<()> {
    svc.remove_job(&job_id)
}

#[tauri::command]
async fn clear_history(svc: State<'_, AppService>) -> AppResult<()> {
    svc.clear_history()
}

#[tauri::command]
async fn hold_queue(svc: State<'_, AppService>) -> AppResult<QueueSnapshot> {
    svc.hold_queue()
}

#[tauri::command]
async fn resume_queue(svc: State<'_, AppService>) -> AppResult<QueueSnapshot> {
    svc.resume_queue()
}

#[tauri::command]
async fn get_logs(svc: State<'_, AppService>) -> AppResult<Vec<LogLine>> {
    Ok(svc.logs())
}

#[tauri::command]
async fn diagnostics_report(svc: State<'_, AppService>, include_identifiers: bool) -> AppResult<DiagnosticsReport> {
    svc.diagnostics_report(include_identifiers).await
}

#[tauri::command]
async fn export_diagnostics(window: Window, svc: State<'_, AppService>, include_identifiers: bool) -> AppResult<bool> {
    let Some(path) =
        pick_save(&window, "Export diagnostics", "nuprint-diagnostics.json", "Diagnostics report", "json").await?
    else {
        return Ok(false);
    };
    svc.export_diagnostics(&path, include_identifiers).await.map(|()| true)
}

// ---------------------------------------------------------------- app icon

/// The app icons offered in Settings › Appearance (`src/lib/theme.ts`). The bundle's own icon is the
/// little cube; another choice replaces it in the Dock (macOS) or on the window and taskbar
/// while NuPrint runs. Finder, Launchpad and shortcuts keep the bundle icon.
fn app_icon_png(icon: &str) -> Option<&'static [u8]> {
    match icon {
        "cube" => Some(include_bytes!("../icons/choices/cube.png")),
        "curl" => Some(include_bytes!("../icons/choices/curl.png")),
        "ribbon" => Some(include_bytes!("../icons/choices/ribbon.png")),
        "roll" => Some(include_bytes!("../icons/choices/roll.png")),
        _ => None,
    }
}

#[tauri::command]
async fn set_app_icon(app: AppHandle, icon: String) -> AppResult<()> {
    let png = app_icon_png(&icon).ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "Unknown app icon."))?;
    cfg_select! {
        target_os = "macos" => {
            app.run_on_main_thread(move || set_dock_icon(png)).map_err(AppError::internal)?;
        }
        _ => {
            let image = tauri::image::Image::from_bytes(png).map_err(AppError::internal)?;
            for window in app.webview_windows().values() {
                window.set_icon(image.clone()).map_err(AppError::internal)?;
            }
        }
    }
    Ok(())
}

/// macOS windows have no icon of their own; the Dock shows the application's image.
#[cfg(target_os = "macos")]
fn set_dock_icon(png: &'static [u8]) {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png)) else {
        tracing::warn!("app icon image could not be decoded");
        return;
    };
    // SAFETY: called on the main thread with a valid, retained image.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
}

// ---------------------------------------------------------------- window theme

/// Light or dark as picked in Settings › Appearance, so the native title bar matches the
/// page; no mode follows the system again.
#[tauri::command]
async fn set_window_theme(app: AppHandle, mode: Option<String>) -> AppResult<()> {
    let theme = match mode.as_deref() {
        None => None,
        Some("light") => Some(tauri::Theme::Light),
        Some("dark") => Some(tauri::Theme::Dark),
        Some(_) => return Err(AppError::new(ErrorCode::InvalidInput, "Unknown window theme.")),
    };
    for window in app.webview_windows().values() {
        window.set_theme(theme).map_err(AppError::internal)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- startup

/// Directories searched for the PDFium shared library: bundled resources first.
fn pdfium_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let resources = app.path().resource_dir().map(|r| vec![r.join("pdfium"), r]).unwrap_or_default();
    nuprint_render::pdfium_search_dirs(&resources)
}

/// `nupreview://localhost/<snapshot-id>.png` → the canonical bitmap PNG held in memory;
/// `nupreview://localhost/job-<job-id>.png` → a saved job's first page, while it is kept.
fn preview_response(app: &AppHandle, path: &str) -> Response<Vec<u8>> {
    let id = path.trim_start_matches('/').strip_suffix(".png").unwrap_or_default();
    let service = app.state::<AppService>();
    let png = match id.strip_prefix("job-") {
        Some(job_id) => service.job_page_png(job_id).map(Arc::new),
        None => service.preview_png(id),
    };
    let builder = Response::builder().header(header::CACHE_CONTROL, "no-store");
    match png {
        Some(png) => {
            builder.status(StatusCode::OK).header(header::CONTENT_TYPE, "image/png").body(png.as_ref().clone())
        }
        None => builder.status(StatusCode::NOT_FOUND).body(Vec::new()),
    }
    .unwrap_or_else(|_| Response::new(Vec::new()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,nuprint=debug".into()),
        )
        .init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol(PREVIEW_SCHEME, |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_owned();
            // A job page is read from the store and encoded here, so serve it on a blocking thread
            // rather than the main thread or an async worker.
            tauri::async_runtime::spawn_blocking(move || responder.respond(preview_response(&app, &path)));
        })
        .setup(|app| {
            let base_dir = app.path().app_data_dir()?;
            let handle = app.handle().clone();
            let events: Arc<dyn nuprint_core::EventSink> = {
                let handle = handle.clone();
                Arc::new(move |event: AppEvent| {
                    let _ = handle.emit(EVENT_CHANNEL, event);
                })
            };
            // One NuPrint owns a data folder and its job queue. A further window (for example
            // for a second printer) gets a folder of its own, with its own saved printers.
            let mut window_number = 1;
            let service = loop {
                let data_dir = if window_number == 1 {
                    base_dir.clone()
                } else {
                    base_dir.join(format!("window-{window_number}"))
                };
                let config = ServiceConfig {
                    data_dir: Some(data_dir),
                    pdfium_dirs: pdfium_dirs(&handle),
                    version: app.package_info().version.to_string(),
                };
                // The service spawns its dispatcher on Tauri's Tokio runtime.
                let started = tauri::async_runtime::block_on(async {
                    AppService::start(config, Arc::new(BleTransport::new()), events.clone())
                });
                match started {
                    Err(e) if e.code == ErrorCode::DataDirInUse && window_number < MAX_WINDOWS => window_number += 1,
                    other => break other.map_err(|e| format!("{e}: {}", e.detail.clone().unwrap_or_default()))?,
                }
            };
            if window_number > 1
                && let Some(window) = app.get_webview_window("main")
            {
                let _ = window.set_title(&format!("NuPrint ({window_number})"));
            }
            app.manage(service);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            list_profiles,
            adapter_status,
            start_scan,
            stop_scan,
            inspect_device,
            cancel_inspect,
            open_bluetooth_settings,
            set_app_icon,
            set_window_theme,
            list_printers,
            save_printer,
            get_printer_profile,
            save_printer_profile,
            reset_printer_profile,
            export_printer_profile,
            remove_printer,
            update_printer_settings,
            get_selected_printer,
            select_printer,
            check_printer,
            open_source,
            render_preview,
            save_preview_png,
            enqueue_job,
            list_jobs,
            get_job,
            cancel_job,
            retry_job,
            remove_job,
            clear_history,
            hold_queue,
            resume_queue,
            get_logs,
            diagnostics_report,
            export_diagnostics,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the NuPrint application");

    app.run(|app, event| {
        if let RunEvent::Exit = event
            && let Some(service) = app.try_state::<AppService>()
        {
            // Stop accepting work; cancel an active transfer and persist its outcome
            // (bounded by Bluetooth cleanup). Pending jobs stay for the next start.
            tauri::async_runtime::block_on(service.shutdown(Duration::from_secs(12)));
        }
    });
}
