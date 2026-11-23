//! Windows only: what GNOME gives Obtura on Linux and Windows does its own way. Obtura's icon in
//! the notification area with its menu (`tray`), keys that work in every app (Print Screen…,
//! through `RegisterHotKey`), keeping a pinned picture above the other windows, and the console
//! for `--help` when Obtura is started from one. Windows' own functions are declared here
//! (user32, kernel32, shell32, advapi32): no extra crate.
//!
//! The icon and the keys talk to one hidden window of Obtura's. GTK's main loop already hands
//! every Windows message of the main thread to its window, so this runs on the main thread;
//! the real work is done from an idle callback, outside Windows' message handling.

use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    rc::Rc,
};

type Hwnd = *mut c_void;
type Handle = *mut c_void;
type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;

#[repr(C)]
struct WndClassExW {
    size: u32,
    style: u32,
    proc_: WndProc,
    class_extra: i32,
    window_extra: i32,
    instance: Handle,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu_name: *const u16,
    class_name: *const u16,
    icon_small: Handle,
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

#[repr(C)]
struct NotifyIconDataW {
    size: u32,
    hwnd: Hwnd,
    id: u32,
    flags: u32,
    callback_message: u32,
    icon: Handle,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    timeout_or_version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: Guid,
    balloon_icon: Handle,
}

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(class: *const WndClassExW) -> u16;
    fn CreateWindowExW(ex_style: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: Hwnd, menu: Handle, instance: Handle, param: *mut c_void) -> Hwnd;
    fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn LoadImageW(instance: Handle, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> Handle;
    fn LoadIconW(instance: Handle, name: *const u16) -> Handle;
    fn GetSystemMetrics(index: i32) -> i32;
    fn CreatePopupMenu() -> Handle;
    fn AppendMenuW(menu: Handle, flags: u32, id: usize, text: *const u16) -> i32;
    fn SetMenuDefaultItem(menu: Handle, item: u32, by_position: u32) -> i32;
    fn TrackPopupMenu(menu: Handle, flags: u32, x: i32, y: i32, reserved: i32, hwnd: Hwnd, rect: *const c_void) -> i32;
    fn DestroyMenu(menu: Handle) -> i32;
    fn SetForegroundWindow(hwnd: Hwnd) -> i32;
    fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn GetCursorPos(p: *mut Point) -> i32;
    fn RegisterHotKey(hwnd: Hwnd, id: i32, modifiers: u32, vk: u32) -> i32;
    fn UnregisterHotKey(hwnd: Hwnd, id: i32) -> i32;
    fn SetWindowPos(hwnd: Hwnd, after: Hwnd, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn AttachConsole(pid: u32) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn Shell_NotifyIconW(message: u32, data: *mut NotifyIconDataW) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(key: Handle, subkey: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, size: *mut u32) -> i32;
}

#[link(name = "gtk-4")]
extern "C" {
    fn gdk_win32_surface_get_handle(surface: *mut c_void) -> Hwnd;
}

const WM_NULL: u32 = 0;
const WM_HOTKEY: u32 = 0x0312;
const WM_CONTEXTMENU: u32 = 0x007B;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_APP: u32 = 0x8000;
const TRAY_MESSAGE: u32 = WM_APP + 1;
const NIN_BALLOONUSERCLICK: u32 = 0x0405;
const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIF_MESSAGE: u32 = 1;
const NIF_ICON: u32 = 2;
const NIF_TIP: u32 = 4;
const NIF_INFO: u32 = 0x10;
const NIIF_INFO: u32 = 1;
const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_TOOLWINDOW: u32 = 0x80;
const IMAGE_ICON: u32 = 1;
const SM_CXSMICON: i32 = 49;
const SM_CYSMICON: i32 = 50;
const MF_STRING: u32 = 0;
const MF_SEPARATOR: u32 = 0x800;
const TPM_RIGHTBUTTON: u32 = 2;
const TPM_RETURNCMD: u32 = 0x100;
const TPM_NONOTIFY: u32 = 0x80;
const SWP_NOSIZE: u32 = 1;
const SWP_NOMOVE: u32 = 2;
const SWP_NOACTIVATE: u32 = 0x10;
const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
const RRF_RT_REG_DWORD: u32 = 0x10;

pub const MOD_CONTROL: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
const MOD_NOREPEAT: u32 = 0x4000;
pub const VK_SNAPSHOT: u32 = 0x2C;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `s` into a fixed buffer, cut to fit (the last place stays 0).
fn fill(buffer: &mut [u16], s: &str) {
    let room = buffer.len() - 1;
    for (b, c) in buffer.iter_mut().zip(s.encode_utf16().take(room).chain(std::iter::repeat(0))) {
        *b = c;
    }
}

/// When started from a command prompt, prints there (`--help`, `--version`, `--ocr`…): Obtura
/// is a windowed program, which Windows starts without a console of its own.
pub fn attach_console() {
    // SAFETY: no arguments to get wrong; fails harmlessly without a parent console.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Keeps `window` above every other window (`on`), or lets it go back among them.
pub fn keep_on_top(window: &impl IsA<gtk::Native>, on: bool) -> bool {
    use gtk::glib::translate::ToGlibPtr;
    let Some(surface) = window.surface() else { return false };
    let ptr: *mut gtk::gdk::ffi::GdkSurface = surface.to_glib_none().0;
    // SAFETY: a live surface of this process; Windows checks the handle itself.
    unsafe {
        let hwnd = gdk_win32_surface_get_handle(ptr.cast());
        let after = if on { -1isize } else { -2isize } as Hwnd;
        !hwnd.is_null() && SetWindowPos(hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) != 0
    }
}

/// Whether Windows' own Snipping Tool takes the Print Screen key (Settings › Accessibility ›
/// Keyboard › "Use the Print screen key to open screen capture", on by default in Windows 11).
pub fn print_screen_opens_snipping_tool() -> bool {
    let hkcu = (0x8000_0001u32 as i32) as isize as Handle;
    let (key, value) = (wide("Control Panel\\Keyboard"), wide("PrintScreenKeyForSnippingEnabled"));
    let mut data = 0u32;
    let mut size = 4u32;
    // SAFETY: a 4-byte answer into a 4-byte place.
    let found = unsafe { RegGetValueW(hkcu, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut data as *mut u32).cast(), &mut size) } == 0;
    found && data != 0
}

// --- The hidden window ---

thread_local! {
    static HWND: Cell<Hwnd> = const { Cell::new(std::ptr::null_mut()) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static ON_HOTKEY: RefCell<Option<Rc<dyn Fn(i32)>>> = const { RefCell::new(None) };
}

fn hidden_window() -> Hwnd {
    let existing = HWND.with(Cell::get);
    if !existing.is_null() {
        return existing;
    }
    let class = wide("ObturaMessages");
    // SAFETY: the class and window are made once, with Obtura's own window procedure.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WndClassExW {
            size: std::mem::size_of::<WndClassExW>() as u32,
            style: 0,
            proc_: window_proc,
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: std::ptr::null_mut(),
            cursor: std::ptr::null_mut(),
            background: std::ptr::null_mut(),
            menu_name: std::ptr::null(),
            class_name: class.as_ptr(),
            icon_small: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);
        TASKBAR_CREATED.with(|t| t.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr())));
        let title = wide("Obtura");
        CreateWindowExW(WS_EX_TOOLWINDOW, class.as_ptr(), title.as_ptr(), WS_POPUP, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null_mut())
    };
    if hwnd.is_null() {
        obtura_core::log::error("Windows: the hidden window for the notification-area icon and the keys could not be made");
    }
    HWND.with(|h| h.set(hwnd));
    hwnd
}

unsafe extern "system" fn window_proc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    match msg {
        TRAY_MESSAGE => {
            match lparam as u32 & 0xFFFF {
                WM_LBUTTONUP | NIN_BALLOONUSERCLICK => {
                    glib::idle_add_local_once(tray::clicked);
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    if let Some(choice) = tray::menu(hwnd) {
                        glib::idle_add_local_once(move || tray::chosen(choice));
                    }
                }
                _ => {}
            }
            0
        }
        WM_HOTKEY => {
            let id = wparam as i32;
            if let Some(f) = ON_HOTKEY.with(|h| h.borrow().clone()) {
                glib::idle_add_local_once(move || f(id));
            }
            0
        }
        m if m != 0 && m == TASKBAR_CREATED.with(Cell::get) => {
            // Explorer started again: the icon must be added again.
            glib::idle_add_local_once(tray::added_again);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// --- Keys in every app ---

/// What to do when one of the keys is pressed (its id).
pub fn on_key(f: impl Fn(i32) + 'static) {
    ON_HOTKEY.with(|h| *h.borrow_mut() = Some(Rc::new(f)));
}

/// Makes `modifiers` + `key` (a Windows virtual key) work in every app, as key `id`. Fails when
/// another program has it.
pub fn add_key(id: i32, modifiers: u32, key: u32) -> bool {
    let hwnd = hidden_window();
    // SAFETY: Obtura's own window; Windows checks the rest.
    !hwnd.is_null() && unsafe { RegisterHotKey(hwnd, id, modifiers | MOD_NOREPEAT, key) } != 0
}

pub fn remove_key(id: i32) {
    let hwnd = hidden_window();
    if !hwnd.is_null() {
        // SAFETY: as above.
        unsafe {
            UnregisterHotKey(hwnd, id);
        }
    }
}

// --- The icon in the notification area ---

/// Obtura's icon beside the clock: its tooltip, its menu (right click) and what a left click
/// does. One icon, whose menu changes with what Obtura is doing (see `crate::tray`).
pub mod tray {
    use super::*;

    /// A menu item: its id and label, a separator being `None`.
    pub type Items = Vec<(u32, Option<String>)>;

    struct Icon {
        tooltip: String,
        items: Items,
        /// What a left click chooses.
        default: Option<u32>,
        on_choice: Rc<dyn Fn(u32)>,
    }

    thread_local! {
        static ICON: RefCell<Option<Icon>> = const { RefCell::new(None) };
        static ADDED: Cell<bool> = const { Cell::new(false) };
    }

    fn data(hwnd: Hwnd) -> NotifyIconDataW {
        NotifyIconDataW {
            size: std::mem::size_of::<NotifyIconDataW>() as u32,
            hwnd,
            id: 1,
            flags: 0,
            callback_message: TRAY_MESSAGE,
            icon: std::ptr::null_mut(),
            tip: [0; 128],
            state: 0,
            state_mask: 0,
            info: [0; 256],
            timeout_or_version: 0,
            info_title: [0; 64],
            info_flags: 0,
            guid: Guid(0, 0, 0, [0; 8]),
            balloon_icon: std::ptr::null_mut(),
        }
    }

    /// Obtura's own icon (resource 1 of obtura.exe), small size.
    fn app_icon() -> Handle {
        // SAFETY: loads from Obtura's own program file; falls back to Windows' standard icon.
        unsafe {
            let icon = LoadImageW(GetModuleHandleW(std::ptr::null()), 1 as *const u16, IMAGE_ICON, GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON), 0);
            if icon.is_null() {
                LoadIconW(std::ptr::null_mut(), 32512 as *const u16)
            } else {
                icon
            }
        }
    }

    /// Shows the icon (or changes it) with `tooltip`, `items` and the item a left click chooses.
    pub fn show(tooltip: &str, items: Items, default: Option<u32>, on_choice: impl Fn(u32) + 'static) -> bool {
        ICON.with(|i| *i.borrow_mut() = Some(Icon { tooltip: tooltip.to_string(), items, default, on_choice: Rc::new(on_choice) }));
        put()
    }

    /// Adds the icon, or brings it up to date.
    fn put() -> bool {
        let hwnd = hidden_window();
        if hwnd.is_null() {
            return false;
        }
        let tooltip = ICON.with(|i| i.borrow().as_ref().map(|i| i.tooltip.clone()));
        let Some(tooltip) = tooltip else { return false };
        let mut d = data(hwnd);
        d.flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        d.icon = app_icon();
        fill(&mut d.tip, &tooltip);
        let added = ADDED.with(Cell::get);
        // SAFETY: a filled structure of the size given in it.
        let ok = unsafe { Shell_NotifyIconW(if added { NIM_MODIFY } else { NIM_ADD }, &mut d) } != 0;
        if ok {
            ADDED.with(|a| a.set(true));
        } else if added {
            // Explorer may have lost it: add it again.
            ADDED.with(|a| a.set(false));
            return put();
        }
        ok
    }

    /// Changes only the tooltip (the time of a recording, how far a scrolling capture got).
    pub fn set_tooltip(tooltip: &str) {
        let changed = ICON.with(|i| i.borrow_mut().as_mut().map(|i| i.tooltip = tooltip.to_string()).is_some());
        if changed {
            put();
        }
    }

    /// A short message from the icon (Windows shows it as a notification).
    pub fn tell(title: &str, text: &str) {
        let hwnd = hidden_window();
        if hwnd.is_null() || !ADDED.with(Cell::get) {
            return;
        }
        let mut d = data(hwnd);
        d.flags = NIF_INFO;
        d.info_flags = NIIF_INFO;
        fill(&mut d.info_title, title);
        fill(&mut d.info, text);
        // SAFETY: as in `put`.
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &mut d);
        }
    }

    /// Takes the icon away.
    pub fn remove() {
        ICON.with(|i| *i.borrow_mut() = None);
        if ADDED.with(|a| a.replace(false)) {
            let mut d = data(hidden_window());
            // SAFETY: as in `put`.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &mut d);
            }
        }
    }

    pub(super) fn added_again() {
        ADDED.with(|a| a.set(false));
        put();
    }

    pub(super) fn clicked() {
        let default = ICON.with(|i| i.borrow().as_ref().and_then(|i| i.default));
        if let Some(id) = default {
            chosen(id);
        }
    }

    pub(super) fn chosen(id: u32) {
        let f = ICON.with(|i| i.borrow().as_ref().filter(|i| i.items.iter().any(|(x, l)| *x == id && l.is_some())).map(|i| i.on_choice.clone()));
        if let Some(f) = f {
            f(id);
        }
    }

    /// Shows the menu at the pointer and gives the item chosen (Windows' own menu).
    pub(super) fn menu(hwnd: Hwnd) -> Option<u32> {
        let (items, default) = ICON.with(|i| i.borrow().as_ref().map(|i| (i.items.clone(), i.default)))?;
        // SAFETY: a menu made, shown and destroyed here, for Obtura's own window.
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return None;
            }
            for (id, label) in &items {
                match label {
                    Some(l) => AppendMenuW(menu, MF_STRING, *id as usize, wide(l).as_ptr()),
                    None => AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null()),
                };
            }
            if let Some(d) = default {
                SetMenuDefaultItem(menu, d, 0);
            }
            let mut p = Point { x: 0, y: 0 };
            GetCursorPos(&mut p);
            // Without this the menu would not close when clicking elsewhere.
            SetForegroundWindow(hwnd);
            let choice = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY, p.x, p.y, 0, hwnd, std::ptr::null());
            PostMessageW(hwnd, WM_NULL, 0, 0);
            DestroyMenu(menu);
            (choice > 0).then_some(choice as u32)
        }
    }
}
