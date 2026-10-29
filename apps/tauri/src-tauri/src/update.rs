//! Updates, only when the user asks: Settings → Updates, or the check at start they can turn on.
//! The updater reads the release manifest named in tauri.conf.json and installs a release only
//! when its signature matches the public key there.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use nuprint_core::api::{LogLevel, UpdateInfo};
use nuprint_core::{AppError, AppResult, AppService, ErrorCode, update};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// How long a check or the download may take before it is given up.
const TIMEOUT: Duration = Duration::from_secs(60);

/// How long closing for the installer waits for the queue to stop, as quitting does.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(12);

/// The update the last check found, for `install_update`.
#[derive(Default)]
pub struct PendingUpdate {
    found: Mutex<Option<Update>>,
    /// Set while an install runs, and kept once one has succeeded and NuPrint is restarting.
    installing: AtomicBool,
}

impl PendingUpdate {
    fn found(&self) -> Option<Update> {
        self.found.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

fn failed(what: &str, error: impl std::fmt::Display) -> AppError {
    AppError::new(
        ErrorCode::Internal,
        format!("NuPrint could not {what}. Check the internet connection and try again."),
    )
    .with_detail(error.to_string())
}

fn refuse_while_printing(svc: &AppService) -> AppResult<()> {
    if svc.is_printing() {
        return Err(AppError::new(
            ErrorCode::PrinterBusy,
            "A job is printing. Install the update when it has finished.",
        ));
    }
    Ok(())
}

/// Ask for the newest release; `None` when this is it.
#[tauri::command]
pub async fn check_for_update(app: AppHandle, pending: State<'_, PendingUpdate>) -> AppResult<Option<UpdateInfo>> {
    let exiting = app.clone();
    let updater = app
        .updater_builder()
        .timeout(TIMEOUT)
        // On Windows the updater starts the installer and ends this process itself, without the
        // exit event that stops the queue on a normal quit; stop it here instead.
        .on_before_exit(move || {
            if let Some(svc) = exiting.try_state::<AppService>() {
                tauri::async_runtime::block_on(svc.shutdown(SHUTDOWN_WAIT));
            }
        })
        .build()
        .map_err(|e| failed("check for updates", e))?;
    let found = updater.check().await.map_err(|e| failed("check for updates", e))?;
    let info = found.as_ref().map(|u| update::update_info(&u.version, &u.current_version, u.body.as_deref(), true));
    *pending.found.lock().unwrap_or_else(PoisonError::into_inner) = found;
    Ok(info)
}

/// Download the update the last check found, install it and restart, one install at a time.
/// Refused while a job prints: closing cancels the job, which could leave it half printed.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    svc: State<'_, AppService>,
    pending: State<'_, PendingUpdate>,
) -> AppResult<()> {
    let update = pending.found().ok_or_else(|| AppError::new(ErrorCode::NotFound, "Check for updates first."))?;
    if pending.installing.swap(true, Ordering::AcqRel) {
        return Err(AppError::invalid("An update is already being installed."));
    }
    let result = install(&app, &svc, update).await;
    if result.is_err() {
        pending.installing.store(false, Ordering::Release);
    }
    result
}

async fn install(app: &AppHandle, svc: &AppService, update: Update) -> AppResult<()> {
    // Saves a download that could not be installed; the hold below is what decides.
    refuse_while_printing(svc)?;
    // The updater checks the signature, including that it names this version.
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| failed("download the update", e))?;
    // Refused if a queued job started during the download; otherwise no job starts until the
    // install fails and the hold is dropped.
    let hold = svc.hold_for_update()?;
    svc.log(LogLevel::Info, format!("Installing NuPrint {}", update.version));
    // On Windows this does not return: the updater runs `on_before_exit` from a blocking thread,
    // where waiting for the queue to stop is allowed, then the installer restarts NuPrint.
    tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await.map_err(AppError::internal)?.map_err(
        |e| {
            // On Windows the queue has already stopped for the installer, and only a restart starts it.
            let restart = if cfg!(windows) { " Restart NuPrint to print again." } else { "" };
            AppError::new(ErrorCode::Internal, format!("NuPrint could not install the update.{restart}"))
                .with_detail(e.to_string())
        },
    )?;
    // Elsewhere the new version is in place; restarting quits through the exit event, which stops
    // the queue as a normal quit does. Waiting jobs start in the new version.
    hold.keep();
    app.request_restart();
    Ok(())
}

/// Open the page of the release the last check found in the browser. The web view never names
/// the address, so this opens nothing else.
#[tauri::command]
pub async fn open_release_page(pending: State<'_, PendingUpdate>) -> AppResult<()> {
    let version = pending.found().map(|u| u.version);
    let version = version.ok_or_else(|| AppError::new(ErrorCode::NotFound, "Check for updates first."))?;
    webbrowser::open(&update::release_url(&version)).map_err(AppError::internal)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_updater_trusts_the_release_signing_key() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["plugins"]["updater"]["pubkey"], nuprint_core::update::PUBLIC_KEY);
    }
}
