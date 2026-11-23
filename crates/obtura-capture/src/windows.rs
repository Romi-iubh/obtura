//! Screen capture on Windows: the screen's own pixels, read through GDI (`BitBlt`), in physical
//! pixels. Windows asks no question and shows no flash, so a still picture of the screen is
//! simply taken (no screen-sharing session as on GNOME). Windows' own functions are declared
//! here directly (user32, gdi32, dwmapi), so no extra crate is needed.
//!
//! A "screen-sharing session" ([`Cast`]) on Windows is only a list of areas of the screen:
//! the whole desktop, or one window. Its frames are read from the screen when asked.
//!
//! Windows tells an app where every screen and every window is, so a window is picked on
//! Obtura's own overlay (the window under the pointer lights up), and a screen is cut from the
//! picture of the desktop by its exact place (see [`screens`], [`windows_on_screen`]).

use crate::{CaptureBackend, CaptureError, CaptureRequest, Cast, CastDone, CastRequest, Done, Place, Shot};
use obtura_image::{Bitmap, Rect};
use std::ffi::c_void;

/// A part of the screen, in physical pixels of the desktop (Windows' virtual screen: the
/// first screen's top-left corner is (0, 0); a screen on its left has negative x).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

type Handle = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct WinRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 1],
}

#[repr(C)]
struct MonitorInfo {
    size: u32,
    monitor: WinRect,
    work: WinRect,
    flags: u32,
    /// The screen's name (`\\.\DISPLAY1`), UTF-16, ends with 0 (MONITORINFOEXW).
    device: [u16; 32],
}

impl MonitorInfo {
    fn new() -> Self {
        MonitorInfo { size: std::mem::size_of::<MonitorInfo>() as u32, monitor: WinRect::default(), work: WinRect::default(), flags: 0, device: [0; 32] }
    }
}

type MonitorEnum = unsafe extern "system" fn(Handle, Handle, *mut WinRect, isize) -> i32;

#[link(name = "user32")]
extern "system" {
    fn GetDC(hwnd: Handle) -> Handle;
    fn ReleaseDC(hwnd: Handle, dc: Handle) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn EnumDisplayMonitors(dc: Handle, clip: *const WinRect, callback: MonitorEnum, data: isize) -> i32;
    fn GetMonitorInfoW(monitor: Handle, info: *mut MonitorInfo) -> i32;
    fn GetWindowTextW(hwnd: Handle, text: *mut u16, max: i32) -> i32;
    fn GetClassNameW(hwnd: Handle, name: *mut u16, max: i32) -> i32;
    fn GetTopWindow(hwnd: Handle) -> Handle;
    fn GetWindow(hwnd: Handle, cmd: u32) -> Handle;
    fn IsWindowVisible(hwnd: Handle) -> i32;
    fn IsIconic(hwnd: Handle) -> i32;
    fn GetWindowThreadProcessId(hwnd: Handle, pid: *mut u32) -> u32;
    fn GetWindowLongPtrW(hwnd: Handle, index: i32) -> isize;
    fn GetWindowRect(hwnd: Handle, rect: *mut WinRect) -> i32;
    fn GetWindowTextLengthW(hwnd: Handle) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(dc: Handle) -> Handle;
    fn CreateCompatibleBitmap(dc: Handle, width: i32, height: i32) -> Handle;
    fn SelectObject(dc: Handle, object: Handle) -> Handle;
    fn BitBlt(dst: Handle, x: i32, y: i32, width: i32, height: i32, src: Handle, sx: i32, sy: i32, rop: u32) -> i32;
    fn GetDIBits(dc: Handle, bitmap: Handle, start: u32, lines: u32, bits: *mut c_void, info: *mut BitmapInfo, usage: u32) -> i32;
    fn DeleteObject(object: Handle) -> i32;
    fn DeleteDC(dc: Handle) -> i32;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmGetWindowAttribute(hwnd: Handle, attribute: u32, value: *mut c_void, size: u32) -> i32;
}

#[link(name = "shcore")]
extern "system" {
    fn GetDpiForMonitor(monitor: Handle, kind: i32, x: *mut u32, y: *mut u32) -> i32;
}

const SM_XVIRTUALSCREEN: i32 = 76;
const SM_YVIRTUALSCREEN: i32 = 77;
const SM_CXVIRTUALSCREEN: i32 = 78;
const SM_CYVIRTUALSCREEN: i32 = 79;
const SRCCOPY: u32 = 0x00CC_0020;
/// Includes windows drawn in layers (most of today's windows).
const CAPTUREBLT: u32 = 0x4000_0000;
const GW_HWNDNEXT: u32 = 2;
const GWL_EXSTYLE: i32 = -20;
const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
const WS_EX_NOACTIVATE: isize = 0x0800_0000;
/// Layered and transparent together: a window clicks go through (an overlay drawn over the
/// screen by a graphics driver or a game bar), not one the user points at.
const WS_EX_TRANSPARENT: isize = 0x0000_0020;
const WS_EX_LAYERED: isize = 0x0008_0000;
/// Windows of the desktop itself (its icons and background) and of the taskbar.
const NOT_WINDOWS: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];
const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;
const DWMWA_CLOAKED: u32 = 14;
const MDT_EFFECTIVE_DPI: i32 = 0;

/// The whole desktop (every screen), in physical pixels.
pub fn desktop() -> Area {
    unsafe {
        Area {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as u32,
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as u32,
        }
    }
}

/// The largest screen scale (1.0 = 96 dpi, 1.5 = 144 dpi...).
pub fn largest_scale() -> f64 {
    unsafe extern "system" fn each(monitor: Handle, _dc: Handle, _rect: *mut WinRect, data: isize) -> i32 {
        let largest = &mut *(data as *mut f64);
        let mut info = MonitorInfo::new();
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetMonitorInfoW(monitor, &mut info) != 0 && GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) == 0 {
            *largest = largest.max(dx as f64 / 96.0);
        }
        1
    }
    let mut largest = 1.0f64;
    unsafe {
        EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), each, &mut largest as *mut f64 as isize);
    }
    largest
}

/// Reads `area` of the screen.
pub fn grab(area: Area) -> Result<Bitmap, String> {
    let (w, h) = (area.width as i32, area.height as i32);
    if w <= 0 || h <= 0 {
        return Err("the part of the screen to read is empty".into());
    }
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return Err("Windows does not let Obtura read the screen".into());
        }
        let memory = CreateCompatibleDC(screen);
        let bitmap = if memory.is_null() { std::ptr::null_mut() } else { CreateCompatibleBitmap(screen, w, h) };
        let result = if bitmap.is_null() {
            Err("not enough memory to read the screen".to_string())
        } else {
            let old = SelectObject(memory, bitmap);
            let copied = BitBlt(memory, 0, 0, w, h, screen, area.x, area.y, SRCCOPY | CAPTUREBLT);
            SelectObject(memory, old);
            // A negative height: rows from the top down, as in a Bitmap.
            let header = BitmapInfoHeader { size: std::mem::size_of::<BitmapInfoHeader>() as u32, width: w, height: -h, planes: 1, bit_count: 32, compression: 0, size_image: 0, x_pels_per_meter: 0, y_pels_per_meter: 0, clr_used: 0, clr_important: 0 };
            let mut info = BitmapInfo { header, colors: [0] };
            let mut rgba = vec![0u8; area.width as usize * area.height as usize * 4];
            let lines = GetDIBits(memory, bitmap, 0, area.height, rgba.as_mut_ptr().cast(), &mut info, 0);
            if copied == 0 || lines != h {
                Err("Windows did not give the screen's picture".to_string())
            } else {
                // GDI gives blue, green, red and an unused byte.
                for px in rgba.chunks_exact_mut(4) {
                    px.swap(0, 2);
                    px[3] = 255;
                }
                Ok(Bitmap { width: area.width, height: area.height, rgba })
            }
        };
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        if !memory.is_null() {
            DeleteDC(memory);
        }
        ReleaseDC(std::ptr::null_mut(), screen);
        result
    }
}

/// [`grab`] as a PNG.
pub fn grab_png(area: Area) -> Result<Vec<u8>, CaptureError> {
    let picture = grab(area).map_err(CaptureError::Failed)?;
    obtura_image::encode_png(&picture).map_err(|e| CaptureError::Failed(e.to_string()))
}

/// The screens: each one's name (`\\.\DISPLAY1`) and its part of the desktop, in physical
/// pixels from the desktop's top-left corner (so, its place in a picture from [`grab`] of
/// [`desktop`]).
pub fn screens() -> Vec<Place> {
    unsafe extern "system" fn each(monitor: Handle, _dc: Handle, _rect: *mut WinRect, data: isize) -> i32 {
        let list = &mut *(data as *mut Vec<(String, WinRect)>);
        let mut info = MonitorInfo::new();
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            let n = info.device.iter().position(|c| *c == 0).unwrap_or(info.device.len());
            list.push((String::from_utf16_lossy(&info.device[..n]), info.monitor));
        }
        1
    }
    let mut list: Vec<(String, WinRect)> = Vec::new();
    unsafe {
        EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), each, &mut list as *mut Vec<(String, WinRect)> as isize);
    }
    let d = desktop();
    list.into_iter()
        .filter(|(_, r)| r.right > r.left && r.bottom > r.top)
        .map(|(name, r)| Place { name, rect: Rect::new(r.left - d.x, r.top - d.y, (r.right - r.left) as u32, (r.bottom - r.top) as u32) })
        .collect()
}

/// UTF-16 text Windows wrote into `buf` (`n` units) as a string.
fn text_of(buf: &[u16], n: i32) -> String {
    String::from_utf16_lossy(&buf[..(n.max(0) as usize).min(buf.len())])
}

/// The windows a user can see and point at, the front one first: not Obtura's own (they are
/// see-through while a capture runs), not minimised, hidden, on another virtual desktop
/// (cloaked), a tool window, a click-through overlay, the desktop or the taskbar; each with
/// its title and its edges as drawn (no invisible resize border), in physical pixels of the
/// desktop, as far as it is on the screen.
fn shown_windows() -> Vec<(Area, String)> {
    let own = std::process::id();
    let d = desktop();
    let mut list = Vec::new();
    unsafe {
        let mut hwnd = GetTopWindow(std::ptr::null_mut());
        while !hwnd.is_null() {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let mut cloaked = 0u32;
            DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), 4);
            let click_through = style & WS_EX_LAYERED != 0 && style & WS_EX_TRANSPARENT != 0;
            let candidate = pid != own
                && IsWindowVisible(hwnd) != 0
                && IsIconic(hwnd) == 0
                && cloaked == 0
                && style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) == 0
                && !click_through
                && GetWindowTextLengthW(hwnd) > 0;
            if candidate {
                let mut class = [0u16; 64];
                let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
                let class = text_of(&class, n);
                let mut r = WinRect::default();
                let framed = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut r as *mut WinRect).cast(), std::mem::size_of::<WinRect>() as u32) == 0;
                if !NOT_WINDOWS.contains(&class.as_str()) && (framed || GetWindowRect(hwnd, &mut r) != 0) {
                    // Only what is on the screen.
                    let (x0, y0) = (r.left.max(d.x), r.top.max(d.y));
                    let (x1, y1) = (r.right.min(d.x + d.width as i32), r.bottom.min(d.y + d.height as i32));
                    if x1 - x0 > 1 && y1 - y0 > 1 {
                        let mut title = vec![0u16; (GetWindowTextLengthW(hwnd).max(0) as usize + 1).min(512)];
                        let n = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
                        list.push((Area { x: x0, y: y0, width: (x1 - x0) as u32, height: (y1 - y0) as u32 }, text_of(&title, n)));
                    }
                }
            }
            hwnd = GetWindow(hwnd, GW_HWNDNEXT);
        }
    }
    list
}

/// The windows on the screen, the front one first, with their places in a picture from
/// [`grab`] of [`desktop`] (see [`shown_windows`] for which ones).
pub fn windows_on_screen() -> Vec<Place> {
    let d = desktop();
    shown_windows().into_iter().map(|(a, name)| Place { name, rect: Rect::new(a.x - d.x, a.y - d.y, a.width, a.height) }).collect()
}

/// The window the user was working in: the front one of [`shown_windows`].
pub fn window_behind_obtura() -> Option<Area> {
    shown_windows().into_iter().next().map(|(a, _)| a)
}

/// Captures the screen of a Windows computer.
pub struct WindowsBackend;

impl CaptureBackend for WindowsBackend {
    fn capture(&self, _request: CaptureRequest, done: Done) {
        done(grab_png(desktop()).and_then(Shot::from_png));
    }

    /// The whole desktop as one "shared screen", or the window the user was working in.
    fn screencast(&self, request: CastRequest, done: CastDone) {
        if request.windows {
            return match window_behind_obtura() {
                Some(area) => done(Ok(Cast::screen_areas(vec![area], 1.0, true))),
                None => done(Err(CaptureError::Failed("There is no window to capture: open the window first, then choose A Window again.".into()))),
            };
        }
        done(Ok(Cast::screen_areas(vec![desktop()], largest_scale(), false)));
    }

    fn capture_is_quiet(&self) -> bool {
        true
    }

    fn screens(&self) -> Option<Vec<Place>> {
        Some(screens())
    }

    fn windows(&self) -> Option<Vec<Place>> {
        Some(windows_on_screen())
    }
}
