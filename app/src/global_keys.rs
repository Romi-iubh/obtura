//! Keys that work in every app. GNOME gives an app's own shortcuts (Ctrl+Shift+R…) only to the
//! app that has the focus, so for a key that works everywhere Obtura adds its commands to GNOME's
//! own custom keyboard shortcuts, when the user turns them on in Preferences (and removes them
//! when turned off). GNOME then runs the command even when Obtura is not running; a running
//! Obtura gets it at once (the command is forwarded to it). They show in GNOME's Settings ›
//! Keyboard › Custom Shortcuts, where the keys can be changed.
//!
//! A key already used anywhere in GNOME's settings (GNOME, its extensions, other custom
//! shortcuts) is not taken.

use gtk::{gio, prelude::*};

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const LIST: &str = "custom-keybindings";
const BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";

pub struct Key {
    /// The last part of the settings path (marks the shortcut as Obtura's).
    id: &'static str,
    /// The name GNOME's Settings shows.
    name: &'static str,
    pub command: &'static str,
    pub binding: &'static str,
    /// What it does, for Obtura's own windows.
    pub what: &'static str,
}

pub const KEYS: [Key; 3] = [
    Key { id: "obtura-region", name: "Obtura: capture a region", command: "obtura --region", binding: "<Super><Shift>s", what: "Capture a region" },
    Key { id: "obtura-text", name: "Obtura: copy the text of a region", command: "obtura --ocr", binding: "<Super><Shift>t", what: "Copy the text of a region" },
    Key { id: "obtura-record", name: "Obtura: start or stop recording", command: "obtura --record", binding: "<Super><Shift>r", what: "Start or stop recording an area" },
];

fn path(key: &Key) -> String {
    format!("{BASE}{}/", key.id)
}

fn schema(id: &str) -> Option<gio::SettingsSchema> {
    gio::SettingsSchemaSource::default()?.lookup(id, true)
}

/// Whether this desktop has GNOME's custom shortcuts.
pub fn available() -> bool {
    schema(MEDIA_KEYS).is_some_and(|s| s.has_key(LIST)) && schema(CUSTOM).is_some()
}

fn list() -> Option<(gio::Settings, Vec<String>)> {
    if !available() {
        return None;
    }
    let settings = gio::Settings::new(MEDIA_KEYS);
    let paths = settings.strv(LIST).iter().map(|p| p.to_string()).collect();
    Some((settings, paths))
}

/// A settings path GSettings accepts (anything else in the list is left alone: opening it
/// would stop Obtura).
fn valid_path(p: &str) -> bool {
    p.starts_with('/') && p.ends_with('/') && !p.contains("//") && p.chars().all(|c| c.is_ascii_alphanumeric() || "/-_.".contains(c))
}

/// The keys of Obtura's shortcuts that are on: (what it does, the key as GNOME shows it).
pub fn current() -> Vec<(&'static str, String)> {
    let Some((_, paths)) = list() else { return Vec::new() };
    KEYS.iter()
        .filter(|k| paths.contains(&path(k)))
        .map(|k| (k.what, label(&gio::Settings::with_path(CUSTOM, &path(k)).string("binding"))))
        .collect()
}

pub fn enabled() -> bool {
    !current().is_empty()
}

/// Modifiers a key binding can hold, in the order they are shown.
const MODIFIERS: [&str; 4] = ["Ctrl", "Alt", "Super", "Shift"];

/// A binding as GNOME writes it ("<Shift><Super>s", "<Primary>Print") read as its modifiers
/// (see [`MODIFIERS`]) and its key in lower case. `None` if it holds no key.
fn parse(binding: &str) -> Option<([bool; 4], String)> {
    let mut mods = [false; 4];
    let mut rest = binding.trim();
    while let Some(after) = rest.strip_prefix('<') {
        let (name, tail) = after.split_once('>')?;
        let i = match name.to_ascii_lowercase().as_str() {
            "control" | "ctrl" | "primary" | "ctl" => 0,
            "alt" | "mod1" => 1,
            "super" | "meta" | "hyper" | "mod4" => 2,
            "shift" => 3,
            _ => return None,
        };
        mods[i] = true;
        rest = tail;
    }
    (!rest.is_empty()).then(|| (mods, rest.to_lowercase()))
}

/// "Super+Shift+S" for "<Super><Shift>s".
pub fn label(binding: &str) -> String {
    let Some((mods, key)) = parse(binding) else { return binding.to_string() };
    let mut parts: Vec<String> = MODIFIERS.iter().zip(mods).filter(|(_, on)| *on).map(|(m, _)| m.to_string()).collect();
    let mut chars = key.chars();
    parts.push(chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default());
    parts.join("+")
}

/// The same key, however it is written ("<Shift><Super>S" = "<Super><Shift>s").
fn same_key(a: &str, b: &str) -> bool {
    matches!((parse(a), parse(b)), (Some(x), Some(y)) if x == y)
}

/// Where `binding` is used already in GNOME's settings (a schema and key), apart from
/// Obtura's own shortcuts.
fn used_by(binding: &str, custom: &[String]) -> Option<String> {
    for p in custom.iter().filter(|p| valid_path(p) && !KEYS.iter().any(|k| path(k) == **p)) {
        let s = gio::Settings::with_path(CUSTOM, p);
        if same_key(&s.string("binding"), binding) {
            return Some(format!("the custom shortcut “{}”", s.string("name")));
        }
    }
    let source = gio::SettingsSchemaSource::default()?;
    let (fixed, _) = source.list_schemas(true);
    for id in fixed {
        let Some(schema) = source.lookup(&id, true) else { continue };
        let lists: Vec<_> = schema.list_keys().into_iter().filter(|k| schema.key(k).value_type().as_str() == "as").collect();
        if lists.is_empty() {
            continue;
        }
        let settings = gio::Settings::new(&id);
        for k in lists {
            if settings.strv(&k).iter().any(|v| same_key(v, binding)) {
                return Some(format!("{id} ({k})"));
            }
        }
    }
    None
}

/// Adds Obtura's shortcuts to GNOME's (`on`) or removes them. Returns what was done, for the
/// user: the keys added, and those left out because something else uses them.
pub fn set(on: bool) -> Result<Vec<String>, String> {
    let (settings, mut paths) = list().ok_or("This desktop has no GNOME keyboard shortcuts")?;
    let mut said = Vec::new();
    for key in &KEYS {
        let p = path(key);
        let custom = gio::Settings::with_path(CUSTOM, &p);
        let present = paths.contains(&p);
        if on && !present {
            if let Some(user) = used_by(key.binding, &paths) {
                obtura_core::log::info(&format!("{} not added: {} already uses it", label(key.binding), user));
                said.push(format!("{} is already used, so it was not added", label(key.binding)));
                continue;
            }
            custom.set_string("name", key.name).map_err(|e| e.to_string())?;
            custom.set_string("command", key.command).map_err(|e| e.to_string())?;
            custom.set_string("binding", key.binding).map_err(|e| e.to_string())?;
            paths.push(p);
        } else if !on && present {
            for k in ["name", "command", "binding"] {
                custom.reset(k);
            }
            paths.retain(|x| *x != p);
        }
    }
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    settings.set_strv(LIST, refs.as_slice()).map_err(|e| e.to_string())?;
    gio::Settings::sync();
    Ok(said)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_compared_however_they_are_written() {
        assert!(same_key("<Super><Shift>s", "<Shift><Super>S"));
        assert!(same_key("<Super><Shift>s", "<Shift><Super>s"));
        assert!(!same_key("<Super><Shift>s", "<Super>s"));
        assert!(!same_key("<Super><Shift>s", "<Super><Shift>t"));
        assert!(!same_key("<Super><Shift>s", ""));
        assert!(!same_key("", ""));
        assert!(same_key("<Control>Print", "<Primary>print"));
        assert!(!same_key("<Weird>s", "<Weird>s"));
    }

    #[test]
    fn keys_read_as_people_write_them() {
        assert_eq!(label("<Super><Shift>s"), "Super+Shift+S");
        assert_eq!(label("<Control><Alt>Print"), "Ctrl+Alt+Print");
        assert_eq!(label("<Primary>F1"), "Ctrl+F1");
    }

    /// Read-only check on the real desktop (run by hand): which of Obtura's keys are free, and
    /// that a key GNOME uses is found.
    #[test]
    #[ignore]
    fn real_desktop_keys() {
        assert!(available());
        let (_, paths) = list().unwrap();
        for k in &KEYS {
            println!("{}: {:?}", label(k.binding), used_by(k.binding, &paths));
        }
        let print = used_by("Print", &paths);
        println!("Print: {print:?}");
        assert!(print.is_some());
    }

    /// Turns the keys on and off against GNOME's real schemas, in memory only (run by hand:
    /// `GSETTINGS_BACKEND=memory cargo test -p obtura -- keys_on_and_off --ignored`).
    #[test]
    #[ignore]
    fn keys_on_and_off() {
        assert_eq!(std::env::var("GSETTINGS_BACKEND").as_deref(), Ok("memory"), "never on the real settings");
        assert!(set(true).unwrap().is_empty());
        let on = current();
        assert_eq!(on.iter().map(|(_, k)| k.as_str()).collect::<Vec<_>>(), ["Super+Shift+S", "Super+Shift+T", "Super+Shift+R"]);
        // Twice is the same as once; another custom shortcut is kept.
        let media = gio::Settings::new(MEDIA_KEYS);
        let mut paths: Vec<String> = media.strv(LIST).iter().map(|p| p.to_string()).collect();
        paths.insert(0, format!("{BASE}custom0/"));
        media.set_strv(LIST, paths.iter().map(String::as_str).collect::<Vec<_>>().as_slice()).unwrap();
        gio::Settings::with_path(CUSTOM, &format!("{BASE}custom0/")).set_string("binding", "<Super><Shift>t").unwrap();
        assert!(set(true).unwrap().is_empty());
        assert_eq!(gio::Settings::new(MEDIA_KEYS).strv(LIST).len(), 4);
        assert!(set(false).unwrap().is_empty());
        assert_eq!(gio::Settings::new(MEDIA_KEYS).strv(LIST).iter().map(|p| p.to_string()).collect::<Vec<_>>(), [format!("{BASE}custom0/")]);
        assert!(!enabled());
        // Now Super+Shift+T is someone else's: not taken.
        let notes = set(true).unwrap();
        assert_eq!(notes, ["Super+Shift+T is already used, so it was not added"]);
        assert_eq!(current().len(), 2);
    }

    #[test]
    fn every_shortcut_has_its_own_settings_path() {
        assert!(!valid_path("custom0/") && !valid_path("/a//b/") && !valid_path("/a/b") && !valid_path("/a b/"));
        assert!(valid_path("/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/"));
        for k in &KEYS {
            assert!(valid_path(&path(k)));
            assert!(path(k).starts_with(BASE) && path(k).ends_with('/') && k.id.starts_with("obtura-"));
            assert!(k.command.starts_with("obtura --"), "{}", k.command);
        }
    }
}
