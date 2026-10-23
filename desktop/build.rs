//! Embeds the app icon and version information into `pixmith-desktop.exe` on Windows, so Explorer and the task
//! bar show the Pixmith icon. Does nothing on other platforms.

fn main() {
    println!("cargo:rerun-if-changed=assets/pixmith.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/pixmith.ico");
        if let Err(e) = res.compile() {
            // A missing resource compiler must not break the build; the app just keeps the default icon.
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
