//! A Microsoft Store (MSIX) install: the Store installs its updates, and it keeps its own saved
//! printers and queue, apart from an installer copy's, in a folder Windows removes with the app.

use std::path::PathBuf;

/// The package family name when Windows runs NuPrint from an app package, as a Store install does.
#[cfg(windows)]
fn package_family() -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFamilyName;
    let mut name = [0u16; 256];
    let mut len = name.len() as u32;
    // `len` is the buffer's length in UTF-16 units; Windows writes no more than that, and sets it
    // to the length written, terminator included.
    let result = unsafe { GetCurrentPackageFamilyName(&mut len, name.as_mut_ptr()) };
    (result == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&name[..(len as usize).saturating_sub(1)]))
}

#[cfg(not(windows))]
fn package_family() -> Option<String> {
    None
}

/// Whether this copy came from the Microsoft Store, which installs its updates.
pub fn installed() -> bool {
    package_family().is_some()
}

/// The Store copy's data folder: its package's `LocalState`.
pub fn data_dir() -> Option<PathBuf> {
    let family = package_family()?;
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(local).join("Packages").join(family).join("LocalState"))
}
