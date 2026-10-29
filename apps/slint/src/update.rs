//! Updates, only when the user asks: Settings → Updates, or the check at start they can turn on.
//! cargo-packager's updater reads the Slint release manifest and installs a release only when its
//! signature matches the key the Tauri app trusts too, and only when that signature names the
//! version offered. It installs on Windows and macOS; on Linux the app offers the release page,
//! where the .deb is.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cargo_packager_updater::{Config, Update, UpdaterBuilder};
use minisign_verify::{PublicKey, Signature};
use nuprint_core::api::UpdateInfo;
use nuprint_core::{AppError, AppResult, AppService, ErrorCode, update};

/// The Slint app's manifest beside the Tauri app's `latest.json`, so neither gets the other's installer.
const MANIFEST: &str = "latest-slint.json";

/// Where cargo-packager can install an update in place.
pub(crate) const CAN_INSTALL: bool = cfg!(any(windows, target_os = "macos"));

/// How long a check or the download may take before it is given up.
const TIMEOUT: Duration = Duration::from_secs(60);

/// How long closing for the installer waits for the queue to stop, as quitting does.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(12);

/// Set when an update has replaced the app; `main` starts it once this one has closed.
static RELAUNCH: AtomicBool = AtomicBool::new(false);

fn failed(what: &str, error: impl std::fmt::Display) -> AppError {
    AppError::new(
        ErrorCode::Internal,
        format!("NuPrint could not {what}. Check the internet connection and try again."),
    )
    .with_detail(error.to_string())
}

/// Ask for the newest release; `None` when this is it. Blocks on the network.
pub(crate) fn check() -> AppResult<Option<(UpdateInfo, Update)>> {
    let endpoint = update::latest_release_file(MANIFEST).parse().map_err(AppError::internal)?;
    let config = Config { endpoints: vec![endpoint], pubkey: update::PUBLIC_KEY.into(), windows: None };
    let current = env!("CARGO_PKG_VERSION").parse().map_err(AppError::internal)?;
    let updater = UpdaterBuilder::new(current, config).timeout(TIMEOUT).build().map_err(AppError::internal)?;
    let found = updater.check().map_err(|e| failed("check for updates", e))?;
    Ok(found.map(|u| (update::update_info(&u.version, &u.current_version, u.body.as_deref(), CAN_INSTALL), u)))
}

/// Check that `package` was signed for `offered`, as the Tauri app's `requireSignedVersion` does.
/// The manifest itself is not signed, so without this a tampered one could offer an older signed
/// release as a newer version. The version is read from the signature's trusted comment, which
/// the Tauri CLI writes as `version:X` and which the signature covers.
fn verify_signed_version(package: &[u8], signature: &str, offered: &str, public_key: &str) -> AppResult<()> {
    let rejected = |detail: String| {
        AppError::new(ErrorCode::Internal, "NuPrint did not install the update: its signature is not valid.")
            .with_detail(detail)
    };
    let decode = |b64: &str| {
        STANDARD
            .decode(b64)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(|| rejected("not base64".into()))
    };
    let key = PublicKey::decode(&decode(public_key)?).map_err(|e| rejected(e.to_string()))?;
    let signature = Signature::decode(&decode(signature)?).map_err(|e| rejected(e.to_string()))?;
    key.verify(package, &signature, true).map_err(|e| rejected(e.to_string()))?;
    let signed = signature.trusted_comment().split('\t').find_map(|field| field.strip_prefix("version:"));
    match signed {
        Some(signed) if signed.trim_start_matches('v') == offered.trim_start_matches('v') => Ok(()),
        Some(signed) => Err(rejected(format!("signed for {signed}, offered as {offered}"))),
        None => Err(rejected("the signature names no version".into())),
    }
}

fn refuse_while_printing(service: &AppService) -> AppResult<()> {
    if service.is_printing() {
        return Err(AppError::new(
            ErrorCode::PrinterBusy,
            "A job is printing. Install the update when it has finished.",
        ));
    }
    Ok(())
}

/// Download and install `update`; the caller then closes the window, and `main` starts the new
/// version. Refused while a job prints: closing cancels the job, which could leave it half printed.
pub(crate) async fn install(service: &AppService, update: Update) -> AppResult<()> {
    // Saves a download that could not be installed; the hold below is what decides.
    refuse_while_printing(service)?;
    let download = update.clone();
    let bytes = tokio::task::spawn_blocking(move || download.download())
        .await
        .map_err(AppError::internal)?
        .map_err(|e| failed("download the update", e))?;
    verify_signed_version(&bytes, &update.signature, &update.version, update::PUBLIC_KEY)?;
    // Refused if a queued job started during the download; otherwise no job starts until the
    // install fails and the hold is dropped.
    let hold = service.hold_for_update()?;
    service.log(nuprint_core::api::LogLevel::Info, format!("Installing NuPrint {}", update.version));
    // On Windows the installer replaces the app while it is closed: `install` starts it and ends
    // this process, so the queue stops first, as it does on a normal quit.
    if cfg!(windows) {
        service.shutdown(SHUTDOWN_WAIT).await;
    }
    tokio::task::spawn_blocking(move || update.install(bytes)).await.map_err(AppError::internal)?.map_err(|e| {
        let restart = if cfg!(windows) { " Restart NuPrint to print again." } else { "" };
        AppError::new(ErrorCode::Internal, format!("NuPrint could not install the update.{restart}"))
            .with_detail(e.to_string())
    })?;
    hold.keep();
    RELAUNCH.store(true, Ordering::Relaxed);
    Ok(())
}

/// Start the installed update, after this process has closed its data folder.
pub(crate) fn relaunch_if_installed() {
    if !RELAUNCH.load(Ordering::Relaxed) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    // macOS starts an app bundle through Launch Services, so it gets its Dock icon and permissions.
    let bundle = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"));
    let started = match bundle {
        Some(bundle) if cfg!(target_os = "macos") => {
            std::process::Command::new("/usr/bin/open").arg("-n").arg(bundle).spawn()
        }
        _ => std::process::Command::new(exe).spawn(),
    };
    if let Err(e) = started {
        eprintln!("NuPrint could not start the new version: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway key, and two signatures of PACKAGE made with `tauri signer sign`: one with
    // `--app-version 1.0.0`, one without.
    const KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDMwQzFGOTk4QTAxNUFEMzcKUldRM3JSV2dtUG5CTUphZnpCMTZiUllXU1VKRWxPMEhTVWQrU3FWVlpzTW9KMHVaUVdhTUhjQVoK";
    const PACKAGE: &[u8] = b"NuPrint 1.0.0 package";
    const SIGNED_1_0_0: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRM3JSV2dtUG5CTU9vYUNVS2wycUppNEV6czkrb2sxRUg3UVE3TDkxcEUrS3dYdDZaaUlIamlVU0tiQy9RQUMxVmM4WFd2YkxjNWJwMEhOU1YzVlFqSG4yLy9uU095REEwPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMzE1OTgxCWZpbGU6b2xkLmJpbgl2ZXJzaW9uOjEuMC4wClFSRFo2YzBwSmcwUDlSV2pvYjhxZS92U1EvZFlzekRwUDZER1ppUnRabzZOb3FaWEk3MCsvQ1RWejQrK1FFdWJFWm5yTWdMS25GcTJXQlJMUTVxM0FRPT0K";
    const UNVERSIONED: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRM3JSV2dtUG5CTUhsZ0hZY3ZOaC9nZk8wTHVFaEN3M2tZR09tMlR4WDc1Ym9YTHU4VmFDdlNzWTRMN3VNOHpRZUlwMlgrYjdYOEg3REtIbXdNMlIyWlZYSEE3Wk1SbEFZPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMzE1OTgyCWZpbGU6b2xkLmJpbgo1T21vcXMxdlBUQlFMRWNLY0tWNkJPTFF0MlRmVXZvNFFjekpQcG8wYmoydjBTbVZUNlVHOFowR0wyNlhMemdMdkhOeVphSGNIT01yK3BVOE1tNm5EZz09Cg==";

    #[test]
    fn installs_only_a_package_signed_for_the_offered_version() {
        assert!(verify_signed_version(PACKAGE, SIGNED_1_0_0, "1.0.0", KEY).is_ok());
        assert!(verify_signed_version(PACKAGE, SIGNED_1_0_0, "v1.0.0", KEY).is_ok());
        // An older signed release in a manifest that offers a newer version.
        let error = verify_signed_version(PACKAGE, SIGNED_1_0_0, "1.1.0", KEY).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("signed for 1.0.0, offered as 1.1.0"));
        assert!(verify_signed_version(PACKAGE, UNVERSIONED, "1.0.0", KEY).is_err());
        assert!(verify_signed_version(b"NuPrint 1.0.0 packagf", SIGNED_1_0_0, "1.0.0", KEY).is_err());
        assert!(verify_signed_version(PACKAGE, SIGNED_1_0_0, "1.0.0", update::PUBLIC_KEY).is_err());
    }
}
