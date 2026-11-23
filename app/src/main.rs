//! Obtura: screenshot, recording and annotation tool for GNOME (and Windows).

// On Windows a windowed program: no console window behind Obtura (see `win32::attach_console`).
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod beautify;
mod capture;
mod editor;
mod format;
#[cfg_attr(windows, path = "global_keys_windows.rs")]
mod global_keys;
mod help;
mod home;
mod icons;
mod inspect;
mod library;
mod modules;
mod overlay;
mod pin;
mod preferences;
mod recorder;
mod screens;
mod scroll;
mod tray;
mod trim;
mod tools;
#[cfg(windows)]
mod win32;
mod window;

fn main() -> gtk::glib::ExitCode {
    #[cfg(windows)]
    windows_graphics();
    #[cfg(windows)]
    win32::attach_console();
    app::run()
}

/// Windows: draw with GTK's Cairo renderer (the CPU) and keep GDK away from OpenGL and
/// Vulkan. GTK's default is OpenGL, and on a Windows 11 PC with an NVIDIA driver
/// (31.0.15.3815) Obtura died while opening its first window, inside the driver's
/// `nvoglv64.dll` (Windows Error Reporting, 0xC0000005); the same fix made the owner's
/// Nexus PDF work there. Obtura draws still pictures and marks with Cairo anyway. A value
/// already set in the environment (`GSK_RENDERER=ngl`, say) is left alone, to try the GPU
/// again. Must run first, before GTK starts and before any other thread exists.
#[cfg(windows)]
fn windows_graphics() {
    for (name, value) in [("GSK_RENDERER", "cairo"), ("GDK_DISABLE", "gl,vulkan")] {
        if std::env::var_os(name).is_none() {
            std::env::set_var(name, value);
        }
    }
}
