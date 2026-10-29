//! The Updates screen: checking for a newer release, installing it, and the check at start.

use nuprint_core::AppError;
use nuprint_core::api::UpdateInfo;
use slint::ComponentHandle;

use crate::ctx::Ctx;
use crate::{AppWindow, Data, update};

pub(super) fn wire(ui: &AppWindow, ctx: &Ctx) {
    on!(ui, ctx, on_check_updates, |c, ui| {
        let d = ui.global::<Data>();
        d.set_update_checking(true);
        d.set_update_newest(false);
        d.set_update_error("".into());
        c.check_for_update(false);
    });
    on!(ui, ctx, on_set_check_updates, |c, _ui, on| {
        c.state.lock().unwrap().prefs.check_updates = on;
        c.save_prefs();
    });
    on!(ui, ctx, on_install_update, |c, ui| {
        let d = ui.global::<Data>();
        // One install at a time: the flag lives in `Data`, so leaving the page does not reset it.
        if d.get_update_installing() {
            return;
        }
        let Some(found) = c.state.lock().unwrap().update.clone() else { return };
        d.set_update_installing(true);
        d.set_update_error("".into());
        let c2 = c.clone();
        c.rt.spawn(async move {
            let result = update::install(&c2.service, found).await;
            c2.ui(move |ui| match result {
                // `main` starts the new version once this window and the queue have closed.
                Ok(()) => {
                    let _ = slint::quit_event_loop();
                }
                Err(e) => {
                    let d = ui.global::<Data>();
                    d.set_update_installing(false);
                    d.set_update_error(e.message.into());
                }
            });
        });
    });
    on!(ui, ctx, on_open_release_page, |c, ui| {
        let version = c.state.lock().unwrap().update.as_ref().map(|u| u.version.clone());
        if let Some(version) = version
            && let Err(e) = webbrowser::open(&nuprint_core::update::release_url(&version))
        {
            ui.global::<Data>().set_update_error(format!("NuPrint could not open the browser: {e}").into());
        }
    });
}

impl Ctx {
    /// Ask for the newest release and show it on the Updates screen. The check at start shows only
    /// a found update, as a status line; a failure there stays quiet.
    pub(crate) fn check_for_update(&self, at_start: bool) {
        let c = self.clone();
        self.rt.spawn(async move {
            let result =
                tokio::task::spawn_blocking(update::check).await.unwrap_or_else(|e| Err(AppError::internal(e)));
            let info = match result {
                Ok(found) => keep_found(&mut c.state.lock().unwrap().update, found),
                Err(e) => {
                    if !at_start {
                        c.ui(move |ui| {
                            let d = ui.global::<Data>();
                            d.set_update_checking(false);
                            d.set_update_error(e.message.into());
                        });
                    }
                    return;
                }
            };
            if at_start && let Some(info) = &info {
                c.notify(format!("NuPrint {} is available.", info.version), 0, Some(("See what’s new", "updates")));
            }
            c.ui(move |ui| {
                let d = ui.global::<Data>();
                d.set_update_checking(false);
                if !at_start {
                    d.set_update_newest(info.is_none());
                }
                let (version, notes, can_install) = info.map_or_default(|i| (i.version, i.notes, i.can_install));
                d.set_update_version(version.into());
                d.set_update_notes(notes.into());
                d.set_update_can_install(can_install);
            });
        });
    }
}

/// Keep what a successful check found in place of what an earlier one found, so a release
/// withdrawn since then is no longer offered.
fn keep_found<T>(kept: &mut Option<T>, found: Option<(UpdateInfo, T)>) -> Option<UpdateInfo> {
    let (info, update) = found.unzip();
    *kept = update;
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_that_finds_nothing_forgets_the_update_found_before() {
        let info = nuprint_core::update::update_info("1.2.0", "1.1.0", None, true);
        let mut kept = None;
        assert_eq!(keep_found(&mut kept, Some((info.clone(), "1.2.0"))), Some(info));
        assert_eq!(kept, Some("1.2.0"));
        assert_eq!(keep_found(&mut kept, None), None);
        assert_eq!(kept, None);
    }
}
