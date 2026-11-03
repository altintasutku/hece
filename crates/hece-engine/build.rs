//! Embeds the Windows version resource (product name, version, icon) into this crate's binaries.

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../apps/settings/src-tauri/icons/icon.ico")
        .set("ProductName", "Hece")
        .set("FileDescription", "Hece speech engine")
        .set("CompanyName", "Hece contributors")
        .set("LegalCopyright", "MIT license");
    res.compile()?;
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
