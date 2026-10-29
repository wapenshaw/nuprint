//! The callbacks the screens raise, wired by feature.

use crate::AppWindow;
use crate::ctx::Ctx;

/// Registers one `Data` callback with the context and the window already in hand.
macro_rules! on {
    ($ui:expr, $ctx:expr, $name:ident, |$c:ident, $w:ident $(, $a:ident)*| $body:block) => {{
        let $c = $ctx.clone();
        let weak = $ui.as_weak();
        $ui.global::<Data>().$name(move |$($a),*| {
            let Some($w) = weak.upgrade() else { return };
            $body
        });
    }};
}

mod appearance;
mod diagnostics;
mod dialogs;
mod printers;
mod queue;
mod shell;
mod workspace;

pub(crate) fn wire(ui: &AppWindow, ctx: &Ctx) {
    workspace::wire(ui, ctx);
    shell::wire(ui, ctx);
    printers::wire(ui, ctx);
    dialogs::wire(ui, ctx);
    queue::wire(ui, ctx);
    diagnostics::wire(ui, ctx);
    appearance::wire(ui, ctx);
}
