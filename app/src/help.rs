//! Help for every window: the user guide (F1), Report a Problem, and About with What's
//! New and the licences of what Obtura contains.

use adw::prelude::*;
use gtk::gio;
use std::path::PathBuf;

/// Where Obtura lives: its source code, its releases and its issue tracker.
pub const WEBSITE: &str = "https://github.com/Romi-iubh/obtura";
pub const ISSUES: &str = "https://github.com/Romi-iubh/obtura/issues";
/// The user guide online, for when the copy installed with Obtura is not there.
pub const GUIDE_ONLINE: &str = "https://github.com/Romi-iubh/obtura/blob/main/docs/user-guide.md";

/// The file name of the user guide, in `<prefix>/share/obtura/` (see `app/Cargo.toml`).
const GUIDE: &str = "Obtura-User-Guide.pdf";

/// The AppStream file is the one place the release notes are written; About shows the
/// current version's notes from it.
const METAINFO: &str = include_str!("../../data/com.grasshillsco.Obtura.metainfo.xml");

pub fn install(app: &adw::Application) {
    let guide = gio::SimpleAction::new("help", None);
    guide.connect_activate({
        let app = app.clone();
        move |_, _| open_guide(&app)
    });
    app.add_action(&guide);
    app.set_accels_for_action("app.help", &["F1"]);
    let report = gio::SimpleAction::new("report-problem", None);
    report.connect_activate({
        let app = app.clone();
        move |_, _| open_uri(&app, ISSUES)
    });
    app.add_action(&report);
    // The log file, for sending with a problem report.
    let log = gio::SimpleAction::new("show-log", None);
    log.connect_activate({
        let app = app.clone();
        move |_, _| {
            let file = gio::File::for_path(obtura_core::log::path());
            gtk::FileLauncher::new(Some(&file)).open_containing_folder(Some(&parent(&app)), None::<&gio::Cancellable>, |_| {});
        }
    });
    app.add_action(&log);
    let about = gio::SimpleAction::new("about", None);
    about.connect_activate({
        let app = app.clone();
        move |_, _| about_dialog(&app)
    });
    app.add_action(&about);
}

/// The last section of every main menu: Preferences to About.
pub fn menu_section() -> gio::Menu {
    let m = gio::Menu::new();
    m.append(Some("Preferences"), Some("app.preferences"));
    m.append(Some("Keyboard Shortcuts"), Some("app.shortcuts"));
    m.append(Some("User Guide"), Some("app.help"));
    m.append(Some("Report a Problem…"), Some("app.report-problem"));
    m.append(Some("Show Log File"), Some("app.show-log"));
    m.append(Some("About Obtura"), Some("app.about"));
    m
}

/// The window to put a dialog on: the active one, else the start window.
fn parent(app: &adw::Application) -> gtk::Window {
    app.active_window().unwrap_or_else(|| crate::home::open(app).upcast())
}

/// The installed guide: `/usr/share/obtura/` beside `/usr/bin/obtura` (any prefix);
/// while developing, the one built into `docs/`.
fn guide_path() -> Option<PathBuf> {
    let installed = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.parent()?.join("share/obtura").join(GUIDE)));
    let source = cfg!(debug_assertions).then(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs").join(GUIDE));
    [installed, source].into_iter().flatten().find(|p| p.is_file())
}

/// Opens the guide (a PDF) in the desktop's document viewer; the guide online without it.
fn open_guide(app: &adw::Application) {
    let Some(path) = guide_path() else { return open_uri(app, GUIDE_ONLINE) };
    let win = parent(app);
    let win2 = win.clone();
    gtk::FileLauncher::new(Some(&gio::File::for_path(&path))).launch(Some(&win), None::<&gio::Cancellable>, move |r| {
        if r.is_err() {
            let text = format!("No program on this computer opens PDF files. The guide is here:\n\n{}", path.display());
            let d = adw::AlertDialog::new(Some("The user guide could not be opened"), Some(&text));
            d.add_response("ok", "OK");
            d.present(Some(&win2));
        }
    });
}

/// Opens a web address in the desktop's browser.
fn open_uri(app: &adw::Application, uri: &str) {
    let win = parent(app);
    let (win2, shown) = (win.clone(), uri.to_string());
    gtk::UriLauncher::new(uri).launch(Some(&win), None::<&gio::Cancellable>, move |r| {
        if r.is_err() {
            let d = adw::AlertDialog::new(Some("No browser could open the page"), Some(&shown));
            d.add_response("ok", "OK");
            d.present(Some(&win2));
        }
    });
}

/// The `<description>` of this version's `<release>` in the AppStream file.
fn release_notes() -> Option<&'static str> {
    let start = METAINFO.find(&format!("<release version=\"{}\"", env!("CARGO_PKG_VERSION")))?;
    let release = &METAINFO[start..];
    let release = &release[..release.find("</release>")?];
    let from = release.find("<description>")? + "<description>".len();
    let to = release.find("</description>")?;
    Some(release[from..to].trim())
}

fn about_dialog(app: &adw::Application) {
    let d = adw::AboutDialog::new();
    d.set_application_name("Obtura");
    d.set_application_icon(crate::app::APP_ID);
    d.set_version(env!("CARGO_PKG_VERSION"));
    d.set_developer_name("Rosaire");
    d.set_developers(&["Rosaire"]);
    d.set_copyright("© 2026 Rosaire");
    d.set_license_type(gtk::License::Agpl30);
    d.set_comments("Capture, annotate, redact and record your screen. Everything stays on this computer: no account, no upload, no telemetry.");
    d.set_website(WEBSITE);
    d.set_issue_url(ISSUES);
    // The AGPL asks that the source code be offered with the program.
    d.add_link("Source Code", WEBSITE);
    d.add_link("User Guide (online)", GUIDE_ONLINE);
    d.add_legal_section(
        "Other components",
        None,
        gtk::License::Custom,
        Some(if cfg!(windows) {
            "Obtura contains a number of Rust libraries (among them gtk-rs, image, png, gif and serde). \
             Their copyright notices and licences are in the folder share\\doc\\obtura where Obtura is \
             installed (THIRD-PARTY-LICENSES.txt).\n\n\
             Obtura for Windows comes with GTK, libadwaita, GLib, cairo, Pango, Tesseract and the \
             libraries they need, from the MSYS2 project, each under its own licence (most of them the \
             GNU LGPL); the licences are in the folder share\\licenses, the list in \
             share\\doc\\obtura\\WINDOWS-COMPONENTS.txt."
        } else {
            "Obtura contains a number of Rust libraries (among them gtk-rs, image, png, gif and serde). \
             Their copyright notices and licences are in /usr/share/doc/obtura/copyright and \
             /usr/share/doc/obtura/THIRD-PARTY-LICENSES.txt.\n\n\
             Obtura uses the programs and libraries of this computer, which keep their own licences: \
             GTK, libadwaita, cairo and Pango to draw, GStreamer to record and Tesseract to read text."
        }),
    );
    if let Some(notes) = release_notes() {
        d.set_release_notes_version(env!("CARGO_PKG_VERSION"));
        d.set_release_notes(notes);
    }
    d.present(Some(&parent(app)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What's New in About comes from the AppStream file: each version needs its notes there.
    #[test]
    fn this_version_has_release_notes() {
        let notes = release_notes().expect("a <release> for this version in the metainfo file");
        assert!(notes.starts_with("<p>") && notes.contains("<li>"), "{notes}");
    }
}
