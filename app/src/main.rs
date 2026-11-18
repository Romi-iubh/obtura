//! Obtura: screenshot, recording and annotation tool for GNOME.

mod app;
mod beautify;
mod capture;
mod editor;
mod format;
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
mod scroll;
mod tray;
mod trim;
mod tools;
mod window;

fn main() -> gtk::glib::ExitCode {
    app::run()
}
