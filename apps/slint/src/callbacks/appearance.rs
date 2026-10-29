//! The Appearance screen: theme, accent and icon preferences.

use slint::ComponentHandle;

use crate::ctx::Ctx;
use crate::{AppWindow, Data, Theme};

pub(super) fn wire(ui: &AppWindow, ctx: &Ctx) {
    on!(ui, ctx, on_set_mode_pref, |c, ui, mode| {
        ui.global::<Theme>().set_mode(mode);
        c.state.lock().unwrap().prefs.mode = mode;
        c.save_prefs();
    });
    on!(ui, ctx, on_set_accent, |c, ui, accent| {
        ui.global::<Theme>().set_accent_index(accent);
        c.state.lock().unwrap().prefs.accent = accent;
        c.save_prefs();
    });
    on!(ui, ctx, on_set_icon, |c, ui, icon| {
        ui.global::<Data>().set_brand_icon(icon);
        c.state.lock().unwrap().prefs.icon = icon;
        c.save_prefs();
    });
}
