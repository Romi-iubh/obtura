//! Obtura's icon in the top bar while recording: the time recorded next to a record (or pause)
//! symbol, and a menu with Pause / Resume, Stop and Save, Discard. The same icon counts down a
//! delayed screenshot (Capture Now, Cancel) and runs a scrolling capture (Finish, Cancel). Ubuntu shows such status icons
//! through its AppIndicator extension; Obtura speaks the two D-Bus interfaces that extension reads
//! (`org.kde.StatusNotifierItem` and `com.canonical.dbusmenu`) directly with GIO.
//!
//! The objects are registered once and stay: between recordings the item is "Passive", which
//! hides it.
//!
//! On Windows the same calls drive Obtura's icon in the notification area (`win32::tray`),
//! which stays there while Obtura runs, with a menu of captures when nothing is under way.
#![cfg_attr(windows, allow(dead_code))]

use gtk::gio;
use gtk::glib::{self, variant::ToVariant, Variant, VariantTy};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/StatusNotifierItem/Menu";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

const XML: &str = r#"<node>
 <interface name="org.kde.StatusNotifierItem">
  <property name="Category" type="s" access="read"/>
  <property name="Id" type="s" access="read"/>
  <property name="Title" type="s" access="read"/>
  <property name="Status" type="s" access="read"/>
  <property name="WindowId" type="i" access="read"/>
  <property name="IconName" type="s" access="read"/>
  <property name="IconThemePath" type="s" access="read"/>
  <property name="OverlayIconName" type="s" access="read"/>
  <property name="AttentionIconName" type="s" access="read"/>
  <property name="IconPixmap" type="a(iiay)" access="read"/>
  <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
  <property name="ItemIsMenu" type="b" access="read"/>
  <property name="Menu" type="o" access="read"/>
  <property name="XAyatanaLabel" type="s" access="read"/>
  <property name="XAyatanaLabelGuide" type="s" access="read"/>
  <property name="XAyatanaOrderingIndex" type="u" access="read"/>
  <method name="ContextMenu"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
  <method name="Activate"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
  <method name="SecondaryActivate"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
  <method name="Scroll"><arg name="delta" type="i" direction="in"/><arg name="orientation" type="s" direction="in"/></method>
  <signal name="NewTitle"/>
  <signal name="NewIcon"/>
  <signal name="NewToolTip"/>
  <signal name="NewStatus"><arg name="status" type="s"/></signal>
  <signal name="XAyatanaNewLabel"><arg name="label" type="s"/><arg name="guide" type="s"/></signal>
 </interface>
 <interface name="com.canonical.dbusmenu">
  <property name="Version" type="u" access="read"/>
  <property name="TextDirection" type="s" access="read"/>
  <property name="Status" type="s" access="read"/>
  <property name="IconThemePath" type="as" access="read"/>
  <method name="GetLayout"><arg type="i" name="parentId" direction="in"/><arg type="i" name="recursionDepth" direction="in"/><arg type="as" name="propertyNames" direction="in"/><arg type="u" name="revision" direction="out"/><arg type="(ia{sv}av)" name="layout" direction="out"/></method>
  <method name="GetGroupProperties"><arg type="ai" name="ids" direction="in"/><arg type="as" name="propertyNames" direction="in"/><arg type="a(ia{sv})" name="properties" direction="out"/></method>
  <method name="GetProperty"><arg type="i" name="id" direction="in"/><arg type="s" name="name" direction="in"/><arg type="v" name="value" direction="out"/></method>
  <method name="Event"><arg type="i" name="id" direction="in"/><arg type="s" name="eventId" direction="in"/><arg type="v" name="data" direction="in"/><arg type="u" name="timestamp" direction="in"/></method>
  <method name="EventGroup"><arg type="a(isvu)" name="events" direction="in"/><arg type="ai" name="idErrors" direction="out"/></method>
  <method name="AboutToShow"><arg type="i" name="id" direction="in"/><arg type="b" name="needUpdate" direction="out"/></method>
  <method name="AboutToShowGroup"><arg type="ai" name="ids" direction="in"/><arg type="ai" name="updatesNeeded" direction="out"/><arg type="ai" name="idErrors" direction="out"/></method>
  <signal name="ItemsPropertiesUpdated"><arg type="a(ia{sv})" name="updatedProps"/><arg type="a(ias)" name="removedProps"/></signal>
  <signal name="LayoutUpdated"><arg type="u" name="revision"/><arg type="i" name="parent"/></signal>
 </interface>
</node>"#;

/// What the menu asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Pause,
    Resume,
    Stop,
    Discard,
    /// A delayed screenshot: take it now.
    CaptureNow,
    /// A delayed screenshot or a scrolling capture: give up.
    Cancel,
    /// A scrolling capture: put the picture together now.
    Finish,
}

/// What the icon is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Recording,
    Countdown,
    Scrolling,
}

/// The menu's items: (id, label), a separator being `None`.
fn items(mode: Mode, paused: bool) -> Vec<(i32, Option<&'static str>)> {
    match mode {
        Mode::Recording => vec![(1, Some(if paused { "Resume Recording" } else { "Pause Recording" })), (2, Some("Stop and Save")), (3, None), (4, Some("Discard…"))],
        Mode::Countdown => vec![(5, Some("Take the Screenshot Now")), (3, None), (6, Some("Cancel"))],
        Mode::Scrolling => vec![(7, Some("Finish Scrolling Capture")), (3, None), (6, Some("Cancel"))],
    }
}

fn action_of(id: i32, mode: Mode, paused: bool) -> Option<Action> {
    if !items(mode, paused).iter().any(|(i, l)| *i == id && l.is_some()) {
        return None;
    }
    match id {
        1 if paused => Some(Action::Resume),
        1 => Some(Action::Pause),
        2 => Some(Action::Stop),
        4 => Some(Action::Discard),
        5 => Some(Action::CaptureNow),
        6 => Some(Action::Cancel),
        7 => Some(Action::Finish),
        _ => None,
    }
}

struct State {
    active: bool,
    mode: Mode,
    paused: bool,
    label: String,
    revision: u32,
    on_action: Option<Rc<dyn Fn(Action)>>,
}

struct Tray {
    conn: gio::DBusConnection,
    state: Rc<RefCell<State>>,
    _ids: Vec<gio::RegistrationId>,
}

thread_local! {
    static TRAY: RefCell<Option<Rc<Tray>>> = const { RefCell::new(None) };
}

fn props(pairs: &[(&str, Variant)]) -> Variant {
    let map: HashMap<String, Variant> = pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    map.to_variant()
}

fn item_props(id: i32, label: Option<&str>) -> Variant {
    match (id, label) {
        (0, _) => props(&[("children-display", "submenu".to_variant())]),
        (_, Some(l)) => props(&[("label", l.to_variant()), ("enabled", true.to_variant()), ("visible", true.to_variant())]),
        (_, None) => props(&[("type", "separator".to_variant()), ("visible", true.to_variant())]),
    }
}

/// The menu as dbusmenu's `(ia{sv}av)`.
fn layout(mode: Mode, paused: bool) -> Variant {
    let children: Vec<Variant> = items(mode, paused).into_iter().map(|(id, label)| Variant::from_variant(&Variant::tuple_from_iter([id.to_variant(), item_props(id, label), Variant::array_from_iter_with_type(VariantTy::VARIANT, Vec::<Variant>::new())]))).collect();
    Variant::tuple_from_iter([0i32.to_variant(), item_props(0, None), Variant::array_from_iter_with_type(VariantTy::VARIANT, children)])
}

fn item_property(s: &State, name: &str) -> Variant {
    match name {
        "Category" => "ApplicationStatus".to_variant(),
        "Id" => "obtura".to_variant(),
        "Title" => "Obtura".to_variant(),
        "Status" => (if s.active { "Active" } else { "Passive" }).to_variant(),
        "WindowId" => 0i32.to_variant(),
        "IconName" => match s.mode {
            Mode::Recording if s.paused => "media-playback-pause-symbolic",
            Mode::Recording => "media-record-symbolic",
            Mode::Countdown => "alarm-symbolic",
            Mode::Scrolling => "go-down-symbolic",
        }
        .to_variant(),
        "IconThemePath" | "OverlayIconName" | "AttentionIconName" => "".to_variant(),
        "IconPixmap" => Variant::array_from_iter_with_type(VariantTy::new("(iiay)").expect("type"), Vec::<Variant>::new()),
        "ToolTip" => {
            let what = match s.mode {
                Mode::Recording if s.paused => "Recording paused",
                Mode::Recording => "Recording the screen",
                Mode::Countdown => "Screenshot in",
                Mode::Scrolling => "Scrolling capture: scroll slowly, then Finish",
            };
            Variant::tuple_from_iter(["".to_variant(), Variant::array_from_iter_with_type(VariantTy::new("(iiay)").expect("type"), Vec::<Variant>::new()), "Obtura".to_variant(), format!("{what} · {}", s.label).to_variant()])
        }
        "ItemIsMenu" => true.to_variant(),
        "Menu" => glib::variant::ObjectPath::try_from(MENU_PATH.to_string()).expect("path").to_variant(),
        "XAyatanaLabel" => s.label.to_variant(),
        "XAyatanaLabelGuide" => "Paused 00:00:00".to_variant(),
        "XAyatanaOrderingIndex" => 0u32.to_variant(),
        _ => "".to_variant(),
    }
}

fn menu_property(name: &str) -> Variant {
    match name {
        "Version" => 3u32.to_variant(),
        "TextDirection" => "ltr".to_variant(),
        "Status" => "normal".to_variant(),
        "IconThemePath" => Vec::<String>::new().to_variant(),
        _ => "".to_variant(),
    }
}

fn register(conn: &gio::DBusConnection, state: &Rc<RefCell<State>>) -> Result<Vec<gio::RegistrationId>, String> {
    let node = gio::DBusNodeInfo::for_xml(XML).map_err(|e| e.to_string())?;
    let item_iface = node.lookup_interface(ITEM).ok_or("no item interface")?;
    let menu_iface = node.lookup_interface(MENU).ok_or("no menu interface")?;
    let s1 = state.clone();
    let item = conn
        .register_object(ITEM_PATH, &item_iface)
        .method_call(|_, _, _, _, _, _, invocation| invocation.return_value(None))
        .property(move |_, _, _, _, name| item_property(&s1.borrow(), name))
        .build()
        .map_err(|e| e.to_string())?;
    let s2 = state.clone();
    let menu = conn
        .register_object(MENU_PATH, &menu_iface)
        .method_call(move |_, _, _, _, method, params, invocation| {
            let (mode, paused) = (s2.borrow().mode, s2.borrow().paused);
            match method {
                "GetLayout" => {
                    let revision = s2.borrow().revision;
                    invocation.return_value(Some(&Variant::tuple_from_iter([revision.to_variant(), layout(mode, paused)])));
                }
                "GetGroupProperties" => {
                    let ids: Vec<i32> = params.child_value(0).get().unwrap_or_default();
                    let all = items(mode, paused);
                    let found: Vec<Variant> = ids
                        .iter()
                        .filter_map(|id| if *id == 0 { Some((0, None)) } else { all.iter().find(|(i, _)| i == id).copied() })
                        .map(|(id, label)| Variant::tuple_from_iter([id.to_variant(), item_props(id, label)]))
                        .collect();
                    invocation.return_value(Some(&Variant::tuple_from_iter([Variant::array_from_iter_with_type(VariantTy::new("(ia{sv})").expect("type"), found)])));
                }
                "GetProperty" => {
                    let id: i32 = params.child_value(0).get().unwrap_or(0);
                    let name: String = params.child_value(1).get().unwrap_or_default();
                    let label = items(mode, paused).into_iter().find(|(i, _)| *i == id).and_then(|(_, l)| l);
                    let value = if name == "label" { label.unwrap_or("").to_variant() } else { "".to_variant() };
                    invocation.return_value(Some(&Variant::tuple_from_iter([Variant::from_variant(&value)])));
                }
                "Event" | "EventGroup" => {
                    let events: Vec<(i32, String)> = if method == "Event" {
                        vec![(params.child_value(0).get().unwrap_or(0), params.child_value(1).get().unwrap_or_default())]
                    } else {
                        let list = params.child_value(0);
                        (0..list.n_children()).map(|i| list.child_value(i)).map(|e| (e.child_value(0).get().unwrap_or(0), e.child_value(1).get().unwrap_or_default())).collect()
                    };
                    if method == "Event" {
                        invocation.return_value(None);
                    } else {
                        invocation.return_value(Some(&Variant::tuple_from_iter([Vec::<i32>::new().to_variant()])));
                    }
                    let on_action = s2.borrow().on_action.clone();
                    for (id, kind) in events {
                        if let (Some(f), Some(a), "clicked") = (&on_action, action_of(id, mode, paused), kind.as_str()) {
                            // After this call has returned: the action may change the menu.
                            let f = f.clone();
                            glib::idle_add_local_once(move || f(a));
                        }
                    }
                }
                "AboutToShow" => invocation.return_value(Some(&Variant::tuple_from_iter([false.to_variant()]))),
                "AboutToShowGroup" => invocation.return_value(Some(&Variant::tuple_from_iter([Vec::<i32>::new().to_variant(), Vec::<i32>::new().to_variant()]))),
                _ => invocation.return_value(None),
            }
        })
        .property(|_, _, _, _, name| menu_property(name))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(vec![item, menu])
}

fn emit(conn: &gio::DBusConnection, path: &str, iface: &str, signal: &str, args: Option<Variant>) {
    let _ = conn.emit_signal(None, path, iface, signal, args.as_ref());
}

/// Whether a top bar that shows status icons is there.
fn watcher_present(conn: &gio::DBusConnection) -> bool {
    conn.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner", Some(&(WATCHER,).to_variant()), None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE)
        .ok()
        .and_then(|v| v.child_value(0).get::<bool>())
        .unwrap_or(false)
}

/// Shows the icon (recording, `label` beside it); menu choices go to `on_action`. Returns false
/// if the top bar shows no status icons (then the caller offers another way to pause).
#[cfg(not(windows))]
pub fn show(label: &str, on_action: impl Fn(Action) + 'static) -> bool {
    show_as(Mode::Recording, label, on_action)
}

/// Shows the icon for `mode`, `label` beside it. Returns false without a status-icon host.
#[cfg(not(windows))]
pub fn show_as(mode: Mode, label: &str, on_action: impl Fn(Action) + 'static) -> bool {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return false };
    if !watcher_present(&conn) {
        return false;
    }
    let tray = TRAY.with(|t| t.borrow().clone());
    let tray = match tray {
        Some(t) => t,
        None => {
            let state = Rc::new(RefCell::new(State { active: false, mode, paused: false, label: String::new(), revision: 1, on_action: None }));
            match register(&conn, &state) {
                Ok(ids) => {
                    let t = Rc::new(Tray { conn: conn.clone(), state, _ids: ids });
                    TRAY.with(|x| *x.borrow_mut() = Some(t.clone()));
                    t
                }
                Err(e) => {
                    obtura_core::log::error(&format!("top-bar icon: {e}"));
                    return false;
                }
            }
        }
    };
    {
        let mut s = tray.state.borrow_mut();
        s.active = true;
        s.mode = mode;
        s.paused = false;
        s.label = label.to_string();
        s.revision += 1;
        s.on_action = Some(Rc::new(on_action));
    }
    let name = tray.conn.unique_name().map(|n| n.to_string()).unwrap_or_default();
    let registered = tray
        .conn
        .call_sync(Some(WATCHER), "/StatusNotifierWatcher", WATCHER, "RegisterStatusNotifierItem", Some(&(name,).to_variant()), None, gio::DBusCallFlags::NONE, 3000, gio::Cancellable::NONE)
        .is_ok();
    emit(&tray.conn, ITEM_PATH, ITEM, "NewStatus", Some(("Active",).to_variant()));
    emit(&tray.conn, ITEM_PATH, ITEM, "NewIcon", None);
    emit(&tray.conn, ITEM_PATH, ITEM, "XAyatanaNewLabel", Some((label, "Paused 00:00:00").to_variant()));
    emit(&tray.conn, MENU_PATH, MENU, "LayoutUpdated", Some((tray.state.borrow().revision, 0i32).to_variant()));
    registered
}

/// The time beside the icon, and whether the recording is paused (icon and menu change).
#[cfg(not(windows))]
pub fn update(label: &str, paused: bool) {
    let Some(tray) = TRAY.with(|t| t.borrow().clone()) else { return };
    let changed = {
        let mut s = tray.state.borrow_mut();
        if !s.active {
            return;
        }
        let changed = s.paused != paused;
        s.paused = paused;
        s.label = label.to_string();
        if changed {
            s.revision += 1;
        }
        changed
    };
    emit(&tray.conn, ITEM_PATH, ITEM, "XAyatanaNewLabel", Some((label, "Paused 00:00:00").to_variant()));
    if changed {
        emit(&tray.conn, ITEM_PATH, ITEM, "NewIcon", None);
        emit(&tray.conn, ITEM_PATH, ITEM, "NewToolTip", None);
        emit(&tray.conn, MENU_PATH, MENU, "LayoutUpdated", Some((tray.state.borrow().revision, 0i32).to_variant()));
    }
}

/// Hides the icon (between recordings).
#[cfg(not(windows))]
pub fn hide() {
    let Some(tray) = TRAY.with(|t| t.borrow().clone()) else { return };
    {
        let mut s = tray.state.borrow_mut();
        s.active = false;
        s.on_action = None;
    }
    emit(&tray.conn, ITEM_PATH, ITEM, "NewStatus", Some(("Passive",).to_variant()));
}

#[cfg(windows)]
pub use self::windows::{hide, show, show_as, start, update};

/// Windows: the icon in the notification area. Between captures its menu offers the captures
/// (a left click captures a region); during a countdown, a scrolling capture or a recording it
/// offers that mode's choices (a left click takes the first) and its tooltip shows the label.
#[cfg(windows)]
mod windows {
    use super::*;
    use crate::win32::tray as icon;
    use gtk::prelude::*;

    struct Shown {
        mode: Mode,
        paused: bool,
        label: String,
        on_action: Rc<dyn Fn(Action)>,
    }

    thread_local! {
        static APP: RefCell<Option<adw::Application>> = const { RefCell::new(None) };
        static SHOWN: RefCell<Option<Shown>> = const { RefCell::new(None) };
    }

    /// The menu between captures: (id, label, what it does).
    const IDLE: [(u32, Option<&str>, &str); 11] = [
        (100, Some("Capture a Region"), "capture-region"),
        (101, Some("Capture the Full Screen"), "capture-screen"),
        (102, Some("Capture a Window"), "capture-window"),
        (103, Some("Scrolling Capture"), "capture-scrolling"),
        (104, Some("Copy Text from the Screen"), "capture-text"),
        (0, None, ""),
        (105, Some("Open Obtura"), ""),
        (106, Some("Library"), "library"),
        (107, Some("Preferences"), "preferences"),
        (0, None, ""),
        (108, Some("Quit Obtura"), ""),
    ];

    /// At start: the icon, with the captures' menu.
    pub fn start(app: &adw::Application) {
        APP.with(|a| *a.borrow_mut() = Some(app.clone()));
        app.connect_shutdown(|_| icon::remove());
        idle();
    }

    fn idle() {
        let items = IDLE.iter().map(|(id, l, _)| (*id, l.map(String::from))).collect();
        icon::show("Obtura: click to capture a region, right-click for more", items, Some(100), |id| {
            let Some(app) = APP.with(|a| a.borrow().clone()) else { return };
            match IDLE.iter().find(|(i, _, _)| *i == id) {
                Some((105, _, _)) => app.activate(),
                Some((108, _, _)) => app.quit(),
                Some((_, _, action)) if !action.is_empty() => app.activate_action(action, None),
                _ => {}
            }
        });
    }

    fn put(s: &Shown) -> bool {
        let items: icon::Items = items(s.mode, s.paused).into_iter().map(|(id, l)| (id as u32, l.map(String::from))).collect();
        let first = items.iter().find(|(_, l)| l.is_some()).map(|(id, _)| *id);
        let (mode, paused, f) = (s.mode, s.paused, s.on_action.clone());
        icon::show(&format!("Obtura: {}", s.label), items, first, move |id| {
            if let Some(a) = action_of(id as i32, mode, paused) {
                f(a)
            }
        })
    }

    pub fn show(label: &str, on_action: impl Fn(Action) + 'static) -> bool {
        show_as(Mode::Recording, label, on_action)
    }

    pub fn show_as(mode: Mode, label: &str, on_action: impl Fn(Action) + 'static) -> bool {
        let s = Shown { mode, paused: false, label: label.to_string(), on_action: Rc::new(on_action) };
        let ok = put(&s);
        SHOWN.with(|x| *x.borrow_mut() = Some(s));
        if ok && mode == Mode::Scrolling {
            let text = if crate::global_keys::current().is_empty() {
                "To finish, click Obtura's icon in the notification area."
            } else {
                "To finish, press Print Screen again, or click Obtura's icon in the notification area."
            };
            icon::tell("Scroll slowly through what you selected", text);
        }
        ok
    }

    pub fn update(label: &str, paused: bool) {
        let again = SHOWN.with(|x| {
            let mut x = x.borrow_mut();
            let s = x.as_mut()?;
            let changed = s.paused != paused;
            s.paused = paused;
            s.label = label.to_string();
            Some(changed)
        });
        match again {
            Some(true) => {
                SHOWN.with(|x| x.borrow().as_ref().map(put));
            }
            Some(false) => icon::set_tooltip(&format!("Obtura: {label}")),
            None => {}
        }
    }

    /// Back to the captures' menu.
    pub fn hide() {
        if SHOWN.with(|x| x.borrow_mut().take()).is_some() {
            idle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(props: &Variant, key: &str) -> Option<String> {
        glib::VariantDict::new(Some(props)).lookup_value(key, None).and_then(|v| v.str().map(str::to_string))
    }

    #[test]
    fn the_menu_layout_has_the_dbusmenu_shape() {
        let l = layout(Mode::Recording, false);
        assert_eq!(l.type_().as_str(), "(ia{sv}av)");
        assert_eq!(l.child_value(2).n_children(), 4);
        let first = l.child_value(2).child_value(0).as_variant().unwrap();
        assert_eq!(first.child_value(0).get::<i32>(), Some(1));
        assert_eq!(text(&first.child_value(1), "label"), Some("Pause Recording".into()));
        let paused = layout(Mode::Recording, true).child_value(2).child_value(0).as_variant().unwrap();
        assert_eq!(text(&paused.child_value(1), "label"), Some("Resume Recording".into()));
        let sep = l.child_value(2).child_value(2).as_variant().unwrap();
        assert_eq!(text(&sep.child_value(1), "type"), Some("separator".into()));
    }

    #[test]
    fn menu_items_ask_for_the_right_thing() {
        let r = Mode::Recording;
        assert_eq!(action_of(1, r, false), Some(Action::Pause));
        assert_eq!(action_of(1, r, true), Some(Action::Resume));
        assert_eq!(action_of(2, r, true), Some(Action::Stop));
        assert_eq!(action_of(4, r, false), Some(Action::Discard));
        assert_eq!(action_of(3, r, false), None);
        assert_eq!(action_of(5, Mode::Countdown, false), Some(Action::CaptureNow));
        assert_eq!(action_of(6, Mode::Countdown, false), Some(Action::Cancel));
        assert_eq!(action_of(7, Mode::Scrolling, false), Some(Action::Finish));
        assert_eq!(action_of(6, Mode::Scrolling, false), Some(Action::Cancel));
        // An item of another menu does nothing.
        assert_eq!(action_of(1, Mode::Scrolling, false), None);
        assert_eq!(action_of(7, r, false), None);
    }

    #[test]
    fn item_properties_have_the_types_the_top_bar_expects() {
        let s = State { active: true, mode: Mode::Recording, paused: false, label: "0:42".into(), revision: 1, on_action: None };
        // Every property declared in the XML, with its type, per interface.
        let (item_xml, menu_xml) = XML.split_at(XML.find("com.canonical.dbusmenu").unwrap());
        let declared = |xml: &str| -> Vec<(String, String)> {
            xml.split("<property name=\"").skip(1).map(|p| {
                let name = p.split('"').next().unwrap().to_string();
                let ty = p.split("type=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
                (name, ty)
            }).collect()
        };
        assert_eq!(declared(item_xml).len(), 16);
        for (name, ty) in declared(item_xml) {
            assert_eq!(item_property(&s, &name).type_().as_str(), ty, "{name}");
        }
        for (name, ty) in declared(menu_xml) {
            assert_eq!(menu_property(&name).type_().as_str(), ty, "{name}");
        }
        // The XML is valid for GIO.
        assert!(gio::DBusNodeInfo::for_xml(XML).is_ok());
    }
}

#[cfg(test)]
mod real {
    /// On the real desktop: the top bar accepts Obtura's icon (visible for four seconds).
    /// Run by hand: `cargo test -p obtura real_top_bar -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_top_bar_takes_the_icon() {
        use gtk::gio;
        use gtk::glib;
        assert!(super::show("0:00", |a| println!("menu: {a:?}")), "no status-icon host in the top bar");
        let ml = glib::MainLoop::new(None, false);
        let m = ml.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || m.quit());
        let conn = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
        let me = conn.unique_name().unwrap().to_string();
        glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
            let v = conn
                .call_sync(Some("org.kde.StatusNotifierWatcher"), "/StatusNotifierWatcher", "org.freedesktop.DBus.Properties", "Get", Some(&glib::Variant::tuple_from_iter([glib::variant::ToVariant::to_variant("org.kde.StatusNotifierWatcher"), glib::variant::ToVariant::to_variant("RegisteredStatusNotifierItems")])), None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE)
                .unwrap();
            println!("registered items: {v}");
            assert!(v.to_string().contains(&me), "the top bar did not list Obtura's icon");
            super::update("Paused 0:01", true);
        });
        ml.run();
        super::hide();
        let m2 = glib::MainLoop::new(None, false);
        let m3 = m2.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || m3.quit());
        m2.run();
    }
}
