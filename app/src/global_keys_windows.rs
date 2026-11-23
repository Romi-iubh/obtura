//! Keys that work in every app, on Windows (the GNOME version is `global_keys.rs`): Print
//! Screen captures a region, Shift+Print Screen copies the text of a region, Ctrl+Print Screen
//! captures the whole screen. Windows gives them to Obtura (`RegisterHotKey`) while it runs, so
//! with them on Obtura stays running, its icon in the notification area. They are on unless
//! turned off in Preferences (settings `keys_everywhere`). During a scrolling capture any of
//! them finishes it.
//!
//! Windows 11 gives Print Screen to its own Snipping Tool unless that is turned off in Windows'
//! Settings (see [`snipping_tool_has_print_screen`]).

use crate::win32;
use gtk::prelude::*;
use obtura_core::settings::Settings;
use std::cell::RefCell;

#[allow(dead_code)]
pub struct Key {
    id: i32,
    modifiers: u32,
    /// The app action it runs.
    action: &'static str,
    /// The same, from the command line.
    pub command: &'static str,
    /// The key, written as GNOME writes keys (for [`label`]).
    pub binding: &'static str,
    /// What it does, for Obtura's own windows.
    pub what: &'static str,
}

pub const KEYS: [Key; 3] = [
    Key { id: 1, modifiers: 0, action: "capture-region", command: "obtura --region", binding: "Print", what: "Capture a region" },
    Key { id: 2, modifiers: win32::MOD_SHIFT, action: "capture-text", command: "obtura --ocr", binding: "<Shift>Print", what: "Copy the text of a region" },
    Key { id: 3, modifiers: win32::MOD_CONTROL, action: "capture-screen", command: "obtura --screen", binding: "<Control>Print", what: "Capture the whole screen" },
];

thread_local! {
    /// The keys Windows gave Obtura.
    static TAKEN: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
}

/// Windows always has them.
pub fn available() -> bool {
    true
}

/// "Shift+Print Screen" for "<Shift>Print".
pub fn label(binding: &str) -> String {
    let mut parts = Vec::new();
    if binding.contains("<Control>") {
        parts.push("Ctrl");
    }
    if binding.contains("<Alt>") {
        parts.push("Alt");
    }
    if binding.contains("<Shift>") {
        parts.push("Shift");
    }
    parts.push("Print Screen");
    parts.join("+")
}

/// The keys that work now: (what it does, the key).
pub fn current() -> Vec<(&'static str, String)> {
    let taken = TAKEN.with(|t| t.borrow().clone());
    KEYS.iter().filter(|k| taken.contains(&k.id)).map(|k| (k.what, label(k.binding))).collect()
}

/// Whether they are on (in the settings).
pub fn enabled() -> bool {
    Settings::load().keys_everywhere
}

/// Whether Windows' Snipping Tool takes Print Screen (then Obtura's Print Screen does nothing).
pub fn snipping_tool_has_print_screen() -> bool {
    win32::print_screen_opens_snipping_tool()
}

/// Windows' Settings page where the Snipping Tool can let go of Print Screen.
pub const KEYBOARD_SETTINGS: &str = "ms-settings:easeofaccess-keyboard";

fn take_keys() -> Vec<String> {
    let mut said = Vec::new();
    for k in &KEYS {
        if TAKEN.with(|t| t.borrow().contains(&k.id)) {
            continue;
        }
        if win32::add_key(k.id, k.modifiers, win32::VK_SNAPSHOT) {
            TAKEN.with(|t| t.borrow_mut().push(k.id));
        } else {
            obtura_core::log::info(&format!("{} not taken: another program has it", label(k.binding)));
            said.push(format!("{} is used by another program", label(k.binding)));
        }
    }
    if snipping_tool_has_print_screen() {
        said.push("Print Screen opens Windows' Snipping Tool: turn that off in Windows Settings › Accessibility › Keyboard".into());
    }
    said
}

fn let_go() {
    for id in TAKEN.with(|t| std::mem::take(&mut *t.borrow_mut())) {
        win32::remove_key(id);
    }
}

/// Turns the keys on or off (and remembers it). Returns notes for the user (keys another
/// program has).
pub fn set(on: bool) -> Result<Vec<String>, String> {
    let mut settings = Settings::load();
    settings.keys_everywhere = on;
    settings.save().map_err(|e| e.to_string())?;
    if !on {
        let_go();
        return Ok(Vec::new());
    }
    let said = take_keys();
    if let Some(app) = gtk::gio::Application::default().and_downcast::<adw::Application>() {
        crate::capture::keep_running(&app);
    }
    Ok(said)
}

/// At start: what the keys do, and the keys themselves when they are on. Obtura then stays
/// running for them once a window was opened (`app::open`) or when started at sign-in
/// (`--gapplication-service`); a one-time command (`obtura --save`) still ends when done.
pub fn start(app: &adw::Application) {
    let a = app.clone();
    win32::on_key(move |id| {
        if crate::scroll::is_running() {
            return crate::scroll::finish();
        }
        if let Some(k) = KEYS.iter().find(|k| k.id == id) {
            a.activate_action(k.action, None);
        }
    });
    if enabled() {
        take_keys();
        if app.flags().contains(gtk::gio::ApplicationFlags::IS_SERVICE) {
            crate::capture::keep_running(app);
        }
    }
}
