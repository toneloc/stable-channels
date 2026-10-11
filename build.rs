// Embed the Windows app icon as a resource in the .exe so Explorer, the
// taskbar (before the eframe window opens), and any shortcuts show the
// branded icon instead of the default Rust executable icon.
//
// Skips silently on non-Windows targets and when the .ico file is missing,
// so this never blocks a Mac / Linux build.

fn main() {
    #[cfg(target_os = "windows")]
    {
        // Path to a multi-resolution .ico file. Generate from photos/sc-icon-egui.png with:
        //   magick photos/sc-icon-egui.png \
        //          -define icon:auto-resize=256,128,64,48,32,16 \
        //          assets/sc-icon.ico
        // (or use the Python Pillow one-liner in the project README).
        let icon_path = "assets/sc-icon.ico";

        if std::path::Path::new(icon_path).exists() {
            let mut res = winres::WindowsResource::new();
            res.set_icon(icon_path);
            if let Err(e) = res.compile() {
                // Don't fail the build — print a warning and ship without the icon.
                println!("cargo:warning=Failed to embed Windows icon: {e}");
            }
            // Re-run if the icon file changes.
            println!("cargo:rerun-if-changed={icon_path}");
        } else {
            println!(
                "cargo:warning=No {icon_path} found — Windows .exe will use the default icon. See build.rs for how to generate one."
            );
        }
    }
}
