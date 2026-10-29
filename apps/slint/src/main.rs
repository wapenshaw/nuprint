//! NuPrint drawn with Slint. It uses the same `AppService` as the Tauri app and the CLI, so
//! scanning, printers, rendering, the job queue and diagnostics behave identically; only the
//! screens are different. The screens are in `ui/` and hold no logic: `Data` carries their state
//! and actions, and the modules here fill it and answer it: `bootstrap` starts the app, `ctx` and `sync`
//! carry service data to the screens, `events` folds service events in, and `callbacks` answers the
//! screens.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod bootstrap;
mod callbacks;
mod ctx;
mod events;
mod labels;
mod preview;
mod state;
mod sync;
mod view;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    bootstrap::run()
}
