use std::path::Path;
use std::{env, fs};

fn main() {
    stage_macos_pdfium();
    tauri_build::build()
}

/// The macOS bundle takes PDFium from `resources/pdfium`. Stage the library for the architecture
/// being built there, so one config serves both Apple silicon and Intel builds.
fn stage_macos_pdfium() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let platform = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "mac-arm64",
        _ => "mac-x64",
    };
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("set by Cargo");
    let source = Path::new(&manifest).join("../../../vendor/pdfium").join(platform);
    let staged = Path::new(&manifest).join("resources/pdfium");
    fs::create_dir_all(&staged).expect("create resources/pdfium");
    for (from, to) in [("lib/libpdfium.dylib", "libpdfium.dylib"), ("LICENSE", "LICENSE")] {
        let from = source.join(from);
        println!("cargo:rerun-if-changed={}", from.display());
        let bytes = fs::read(&from)
            .unwrap_or_else(|_| panic!("{} is missing; run scripts/fetch-pdfium.sh {platform}", from.display()));
        // Rewrite only on change, so `tauri dev` does not see a new file after every build.
        let to = staged.join(to);
        if fs::read(&to).ok().as_deref() != Some(bytes.as_slice()) {
            fs::write(&to, bytes).expect("stage PDFium");
        }
    }
}
