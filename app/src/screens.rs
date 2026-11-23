//! Full screen with several screens: which one the picture keeps. The first time, Obtura asks on
//! the overlay (click a screen, or All Screens) and remembers the answer (setting `full_screen`)
//! unless "Remember my choice" was unticked; Preferences › Full Screen changes it. With one
//! screen nothing is asked.
//!
//! A screen is found again by the name the system gives it (GNOME: its connector, "DP-7";
//! Windows: `\\.\DISPLAY1`), or by where it is on the desktop when it has none.

use crate::{
    capture::{deliver, fail, Done, Request},
    overlay::{self, ScreenChoice},
};
use obtura_capture::{CaptureBackend, CaptureError, Place};
use obtura_core::settings::{Settings, FULL_SCREEN_ALL, FULL_SCREEN_ASK};
use obtura_image::{Bitmap, Rect};
use std::{cell::RefCell, rc::Rc};

/// One screen of the desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    /// What the settings keep to find this screen again.
    pub id: String,
    /// "Screen 1 · DP-7 · 1920 × 1080".
    pub label: String,
    /// Its part of a picture of the whole desktop (physical pixels).
    pub pixels: Rect,
}

/// The screens, in the order of the overlay's monitors, with their parts of a `width` x `height`
/// picture of the whole desktop. Where the system gives the screens' exact places (Windows,
/// where screens may have different scales), those are used; otherwise the picture follows
/// GTK's layout of the monitors (GNOME).
pub fn list(backend: &dyn CaptureBackend, width: u32, height: u32) -> Result<Vec<Screen>, String> {
    let desktop = overlay::desktop()?;
    let names = overlay::monitor_names();
    let dw = desktop.screens.iter().map(|s| s.x + s.w).fold(1.0, f64::max);
    let dh = desktop.screens.iter().map(|s| s.y + s.h).fold(1.0, f64::max);
    let mapped: Vec<Rect> = desktop
        .screens
        .iter()
        .map(|s| overlay::Area { x: s.x, y: s.y, w: s.w, h: s.h, desktop: (dw, dh) }.in_pixels(width, height).unwrap_or(Rect::new(0, 0, width, height)))
        .collect();
    let exact = backend.screens().unwrap_or_default();
    let matched = match_places(&mapped, &exact);
    Ok(mapped
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let (name, pixels) = match matched[i].and_then(|j| exact.get(j)) {
                Some(p) => (p.name.clone(), p.rect),
                None => (names.get(i).cloned().unwrap_or_default(), *m),
            };
            let s = &desktop.screens[i];
            let id = if name.is_empty() { format!("at {},{} {}x{}", s.x, s.y, s.w, s.h) } else { name.clone() };
            Screen { id, label: label(i, &name, pixels), pixels }
        })
        .collect())
}

/// "Screen 1 · DP-7 · 1920 × 1080" (Windows' `\\.\` before a screen's name is left out).
fn label(i: usize, name: &str, pixels: Rect) -> String {
    let name = name.trim_start_matches("\\\\.\\");
    if name.is_empty() {
        format!("Screen {} · {} × {}", i + 1, pixels.w, pixels.h)
    } else {
        format!("Screen {} · {} · {} × {}", i + 1, name, pixels.w, pixels.h)
    }
}

/// For each of `mapped` (a screen's part of the picture as GTK lays the screens out), the place
/// of `exact` (as the system gives it) that is the same screen: of every way of giving each
/// screen a different place (or none), the one where they overlap most in all.
fn match_places(mapped: &[Rect], exact: &[Place]) -> Vec<Option<usize>> {
    let overlap = |a: &Rect, b: &Rect| {
        let w = (a.x as i64 + a.w as i64).min(b.x as i64 + b.w as i64) - (a.x as i64).max(b.x as i64);
        let h = (a.y as i64 + a.h as i64).min(b.y as i64 + b.h as i64) - (a.y as i64).max(b.y as i64);
        if w > 0 && h > 0 { w * h } else { 0 }
    };
    let o: Vec<Vec<i64>> = mapped.iter().map(|m| exact.iter().map(|p| overlap(m, &p.rect)).collect()).collect();
    fn search(i: usize, o: &[Vec<i64>], used: &mut [bool], now: &mut [Option<usize>], sum: i64, best: &mut (i64, Vec<Option<usize>>)) {
        if i == o.len() {
            if sum > best.0 {
                *best = (sum, now.to_vec());
            }
            return;
        }
        search(i + 1, o, used, now, sum, best);
        for j in 0..used.len() {
            if !used[j] && o[i][j] > 0 {
                used[j] = true;
                now[i] = Some(j);
                search(i + 1, o, used, now, sum + o[i][j], best);
                now[i] = None;
                used[j] = false;
            }
        }
    }
    let mut best = (0, vec![None; mapped.len()]);
    // A few screens at most; past eight, the layout of the picture is used as it is.
    if mapped.len() <= 8 && exact.len() <= 8 {
        search(0, &o, &mut vec![false; exact.len()], &mut vec![None; mapped.len()], 0, &mut best);
    }
    best.1
}

/// What `choice` (the setting) takes among `screens`: `Some(None)` every screen, `Some(Some(i))`
/// screen `i`, `None` ask (asked for, or the screen chosen is not connected now).
fn resolve(choice: &str, screens: &[Screen]) -> Option<Option<usize>> {
    match choice {
        FULL_SCREEN_ASK => None,
        FULL_SCREEN_ALL => Some(None),
        id => screens.iter().position(|s| s.id == id).map(Some),
    }
}

/// Full screen: `full` is the whole desktop; keeps the screen remembered, or asks which.
pub fn take(app: &adw::Application, backend: Rc<dyn CaptureBackend>, full: Bitmap, request: Request, hidden: Vec<gtk::Window>, settings: Settings, done: Done) {
    let screens = match list(&*backend, full.width, full.height) {
        Ok(s) if s.len() > 1 => s,
        _ => return deliver(app, full, &request, hidden, &settings, done),
    };
    match resolve(&settings.full_screen, &screens) {
        Some(None) => return deliver(app, full, &request, hidden, &settings, done),
        Some(Some(i)) => {
            if let Some(picture) = full.crop(screens[i].pixels) {
                return deliver(app, picture, &request, hidden, &settings, done);
            }
        }
        None => {}
    }
    let labels = screens.iter().map(|s| s.label.clone()).collect();
    let full = Rc::new(full);
    let slot = Rc::new(RefCell::new(Some((request, hidden, settings, done))));
    let (app2, full2, slot2) = (app.clone(), full.clone(), slot.clone());
    let shown = overlay::choose_screen(
        &full,
        labels,
        Box::new(move |answer| {
            let Some((request, hidden, settings, done)) = slot2.borrow_mut().take() else { return };
            let Some((choice, remember)) = answer else {
                return fail(&app2, &request, &hidden, CaptureError::Cancelled, done);
            };
            let chosen = match choice {
                ScreenChoice::All => Some(((*full2).clone(), FULL_SCREEN_ALL.to_string())),
                ScreenChoice::One(i) => screens.get(i).and_then(|s| full2.crop(s.pixels).map(|p| (p, s.id.clone()))),
            };
            let Some((picture, id)) = chosen else {
                return fail(&app2, &request, &hidden, CaptureError::Failed("That screen is not in the picture.".into()), done);
            };
            if remember {
                let mut s = Settings::load();
                if s.full_screen != id {
                    s.full_screen = id;
                    if let Err(e) = s.save() {
                        obtura_core::log::warn(&format!("could not keep the screen chosen: {e}"));
                    }
                }
            }
            deliver(&app2, picture, &request, hidden, &settings, done)
        }),
    );
    if let Err(why) = shown {
        if let Some((request, hidden, _, done)) = slot.borrow_mut().take() {
            fail(app, &request, &hidden, CaptureError::Failed(why), done);
        }
    }
}

/// For Preferences: the choices offered (what the setting keeps, what is shown), with the one
/// in use, also when its screen is not connected now.
pub fn choices(backend: &dyn CaptureBackend, current: &str) -> Vec<(String, String)> {
    let mut out = vec![(FULL_SCREEN_ASK.to_string(), "Ask Which Screen".to_string()), (FULL_SCREEN_ALL.to_string(), "All Screens".to_string())];
    // The picture's size is only needed for the sizes shown: physical pixels, at the largest scale.
    let size = overlay::desktop().ok().map(|d| {
        let scale = d.screens.iter().map(|s| s.scale).fold(1.0, f64::max);
        let w = d.screens.iter().map(|s| s.x + s.w).fold(1.0, f64::max);
        let h = d.screens.iter().map(|s| s.y + s.h).fold(1.0, f64::max);
        ((w * scale).round() as u32, (h * scale).round() as u32)
    });
    if let Some(screens) = size.and_then(|(w, h)| list(backend, w, h).ok()) {
        out.extend(screens.into_iter().map(|s| (s.id, s.label)));
    }
    if !out.iter().any(|(id, _)| id == current) {
        out.push((current.to_string(), format!("{} (not connected)", current.trim_start_matches("\\\\.\\"))));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(name: &str, x: i32, y: i32, w: u32, h: u32) -> Place {
        Place { name: name.into(), rect: Rect::new(x, y, w, h) }
    }

    #[test]
    fn each_screen_finds_its_own_place() {
        // Windows, a laptop at 200 % (2880 x 1800) and a screen at 100 % beside it. GTK lays
        // them out as 1440 x 900 and 1920 x 1080 side by side; scaled to the 4800 px picture
        // that is not exact, but each screen still finds its own place.
        let mapped = [Rect::new(0, 0, 2057, 1285), Rect::new(2057, 0, 2743, 1542)];
        let exact = [place("\\\\.\\DISPLAY2", 2880, 0, 1920, 1080), place("\\\\.\\DISPLAY1", 0, 0, 2880, 1800)];
        assert_eq!(match_places(&mapped, &exact), vec![Some(1), Some(0)]);
        // Both overlap the laptop's place most, but together they overlap most when the second
        // one is the screen beside it.
        let mapped = [Rect::new(0, 0, 1440, 900), Rect::new(1440, 0, 2000, 1080)];
        assert_eq!(match_places(&mapped, &exact), vec![Some(1), Some(0)]);
        // Two of GTK's screens on the same place: one gets it, the other keeps GTK's layout.
        assert_eq!(match_places(&[Rect::new(0, 0, 100, 100), Rect::new(0, 0, 100, 100)], &[place("x", 0, 0, 100, 100)]).iter().filter(|m| m.is_some()).count(), 1);
        // No exact places (GNOME): none found.
        assert_eq!(match_places(&mapped, &[]), vec![None, None]);
        // A place nowhere near: not taken.
        assert_eq!(match_places(&[Rect::new(0, 0, 10, 10)], &[place("x", 50, 50, 10, 10)]), vec![None]);
    }

    #[test]
    fn the_setting_takes_the_screen_named_or_asks() {
        let screens = vec![
            Screen { id: "DP-7".into(), label: String::new(), pixels: Rect::new(0, 0, 1920, 1080) },
            Screen { id: "HDMI-1".into(), label: String::new(), pixels: Rect::new(1920, 0, 1920, 1080) },
        ];
        assert_eq!(resolve(FULL_SCREEN_ASK, &screens), None);
        assert_eq!(resolve(FULL_SCREEN_ALL, &screens), Some(None));
        assert_eq!(resolve("HDMI-1", &screens), Some(Some(1)));
        // The screen chosen was unplugged: Obtura asks again.
        assert_eq!(resolve("DP-1", &screens), None);
    }

    #[test]
    fn a_screen_is_said_by_its_number_name_and_size() {
        assert_eq!(label(0, "DP-7", Rect::new(0, 0, 1920, 1080)), "Screen 1 · DP-7 · 1920 × 1080");
        assert_eq!(label(1, "\\\\.\\DISPLAY2", Rect::new(1920, 0, 2560, 1440)), "Screen 2 · DISPLAY2 · 2560 × 1440");
        assert_eq!(label(2, "", Rect::new(0, 0, 800, 600)), "Screen 3 · 800 × 600");
    }
}
