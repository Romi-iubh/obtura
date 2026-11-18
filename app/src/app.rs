//! The application object: command line, actions and the choice of capture backend.
//!
//! `obtura --region` and `obtura --screen` start a capture. If Obtura is already running
//! the arguments are forwarded to that instance, so a shortcut starts a capture at once.

use crate::{
    capture::{self, Mode, Output, Outcome, Request},
    home, icons, preferences, recorder,
};
use adw::prelude::*;
use gtk::{gio, glib};
use obtura_capture::{CaptureBackend, FakeBackend, PortalBackend};
use std::{cell::Cell, ffi::OsString, path::PathBuf, rc::Rc};

pub const APP_ID: &str = "com.grasshillsco.Obtura";

thread_local! {
    /// How the last command-line capture ended. When this process is the running instance,
    /// GLib reads the exit status as soon as the handler returns, so a capture that ends
    /// later (after the delay, or after the overlay) is reported through `run`'s result.
    static STATUS: Cell<Option<i32>> = const { Cell::new(None) };
}

/// GTK, GLib and GStreamer-side warnings and errors go to the log file too (and still to the
/// terminal or the system journal, as before).
fn capture_toolkit_messages() {
    glib::log_set_writer_func(|level, fields| {
        let field = |k: &str| fields.iter().find(|f| f.key() == k).and_then(|f| f.value_str()).unwrap_or("").to_string();
        let name = match level {
            glib::LogLevel::Error => Some("FATAL"),
            glib::LogLevel::Critical => Some("CRITICAL"),
            glib::LogLevel::Warning => Some("warning"),
            _ => None,
        };
        if let Some(name) = name {
            let domain = field("GLIB_DOMAIN");
            obtura_core::log::note(name, &format!("{}{}", if domain.is_empty() { String::new() } else { format!("{domain}: ") }, field("MESSAGE")));
        }
        glib::log_writer_default(level, fields)
    });
}

pub fn run() -> glib::ExitCode {
    // --help, --version and wrong options are answered here, before GTK needs a display, so
    // they also work over SSH, in a text console or in a script. (--gapplication-service,
    // from D-Bus activation, is GApplication's own.)
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(Command::Help) => {
            print!("{USAGE}");
            return glib::ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("obtura {}", env!("CARGO_PKG_VERSION"));
            return glib::ExitCode::SUCCESS;
        }
        Err(msg) if !args.iter().any(|a| a.starts_with("--gapplication")) => {
            eprintln!("obtura: {msg}. Try --help.");
            return glib::ExitCode::from(2);
        }
        _ => {}
    }
    let app =adw::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE).build();
    let backend = backend();
    app.connect_startup({
        let backend = backend.clone();
        move |app| {
            // The log first, so that anything going wrong afterwards is in it.
            obtura_core::log::init(capture::now);
            capture_toolkit_messages();
            icons::init();
            // Beautify background pictures are read from Obtura's picture store when needed.
            obtura_scene::set_picture_loader(obtura_core::backgrounds::load);
            gtk::Window::set_default_icon_name(APP_ID);
            install_actions(app, &backend);
            recorder::clean_up_leftovers();
            crate::scroll::clean_up_leftovers();
            // Old captures go, as the settings ask (never the ones marked "keep always").
            crate::library::clean_up(&obtura_core::settings::Settings::load());
        }
    });
    // Started by D-Bus activation (the app grid, a click on the dock icon of a running Obtura).
    app.connect_activate(|app| open(app));
    app.connect_command_line(move |app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().skip(1).map(|a: &OsString| a.to_string_lossy().into_owned()).collect();
        glib::ExitCode::from(handle_args(app, &backend, cmd, &args) as u8)
    });
    let code = app.run();
    match STATUS.get() {
        Some(status) => glib::ExitCode::from(status as u8),
        None => code,
    }
}

const USAGE: &str = "Usage: obtura [OPTIONS]

Capture:
  --region          select a part of the screen
  --screen          capture the whole screen
  --last-region     capture the region chosen last time (asks if there is none)
  --window          capture a window, picked in GNOME's list of windows
  --scroll          scrolling capture: select what scrolls, then scroll it slowly;
                    run it again (or click Obtura's icon in the top bar) to finish
  --delay N         wait N seconds (0-30) before capturing
  --copy            copy the picture to the clipboard, without opening the editor
                    (Obtura then keeps running in the background, which keeps the
                    clipboard; run it with & in a script)
  --save [PATH]     save the picture as a PNG, without opening the editor
                    (no PATH: the save folder, under the naming template)
  --ocr             read the text in the picture, copy it and print it
                    (a region unless --screen or --last-region is given)
  --qr              read the QR codes in the picture, copy and print what they
                    hold (a region unless --screen or --last-region is given)
  --pin             pin the picture on the screen (a region unless --screen...)
  --pick-color      pick a colour on the screen; copies and prints it (#rrggbb)
  --measure         measure on the screen (drag; Enter copies and prints it)
                    (--ocr, --qr, --pick-color and --measure keep Obtura running,
                    like --copy)

Record:
  --record          record an area of the screen (with --screen: a whole screen;
                    with --window: a window picked in GNOME's list; with
                    --all-screens: every shared screen in one video)
                    as set in the Record window (format, sound, countdown).
                    Run it again to stop; the first command then prints the file
                    saved. Clicking Obtura's icon pauses.

Other:
  --library         open the library of past screenshots
  -h, --help        show this text
  --version         show the version

Without an option Obtura opens its start window.
Exit status: 0 done, 1 failed (or no text or code found), 2 wrong options, 3 cancelled.
";

/// What the command line asks for.
#[derive(Debug, PartialEq)]
enum Command {
    Open,
    Library,
    Help,
    Version,
    Capture(Request),
    Record(recorder::What),
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    let (mut mode, mut delay, mut copy, mut save) = (None, 0u32, false, None::<PathBuf>);
    let (mut text, mut pin, mut color, mut record, mut qr, mut ruler) = (false, false, false, false, false, false);
    let (mut window, mut all_screens) = (false, false);
    let mut other = None;
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--region" => mode = Some(Mode::Region),
            "--screen" => mode = Some(Mode::Screen),
            "--last-region" => mode = Some(Mode::LastRegion),
            "--scroll" => mode = Some(Mode::Scrolling),
            "--copy" => copy = true,
            "--ocr" => text = true,
            "--qr" => qr = true,
            "--pin" => pin = true,
            "--pick-color" | "--pick-colour" => color = true,
            "--measure" => ruler = true,
            "--record" => record = true,
            "--window" => window = true,
            "--all-screens" => all_screens = true,
            "--delay" => {
                let n = it.next().ok_or("--delay needs a number of seconds")?;
                delay = n.parse::<u32>().ok().filter(|n| *n <= 30).ok_or_else(|| format!("--delay must be a whole number from 0 to 30, not '{n}'"))?;
            }
            "--save" => {
                // The path is optional: without one, the save folder is used.
                save = Some(match it.peek() {
                    Some(next) if !next.starts_with("--") => PathBuf::from(it.next().expect("peeked")),
                    _ => PathBuf::new(),
                });
            }
            "-h" | "--help" => other = Some(Command::Help),
            "--library" => other = Some(Command::Library),
            "--version" => other = Some(Command::Version),
            unknown => return Err(format!("unknown option '{unknown}'")),
        }
    }
    if let Some(c) = other {
        return Ok(c);
    }
    if record {
        if text || qr || pin || color || ruler || copy || save.is_some() || delay > 0 || mode == Some(Mode::LastRegion) || (window && all_screens) || ((window || all_screens) && mode.is_some()) {
            return Err("--record takes one of --region, --screen, --window or --all-screens (the countdown is set in the Record window)".into());
        }
        return Ok(Command::Record(match (mode, window, all_screens) {
            (_, true, _) => recorder::What::Window,
            (_, _, true) => recorder::What::AllScreens,
            (Some(Mode::Screen), _, _) => recorder::What::Screen,
            _ => recorder::What::Area,
        }));
    }
    if all_screens {
        return Err("--all-screens goes with --record".into());
    }
    if window {
        if mode.is_some() {
            return Err("choose one of --region, --screen, --last-region, --scroll or --window".into());
        }
        mode = Some(Mode::Window);
    }
    let special = [text, qr, pin, color, ruler].iter().filter(|x| **x).count();
    if special > 1 || (special == 1 && (copy || save.is_some())) {
        return Err("choose one of --ocr, --qr, --pin, --pick-color, --measure, or --copy/--save".into());
    }
    if ruler {
        if mode.is_some() {
            return Err("--measure works on the whole screen: no --region, --screen or --scroll".into());
        }
        return Ok(Command::Capture(Request { mode: Mode::Screen, delay, output: Output::Measure }));
    }
    if color {
        if mode.is_some() {
            return Err("--pick-color works on the whole screen: no --region or --screen".into());
        }
        return Ok(Command::Capture(Request { mode: Mode::Screen, delay, output: Output::Color }));
    }
    if text || qr || pin {
        let output = if text { Output::Text } else if qr { Output::Qr } else { Output::Pin };
        return Ok(Command::Capture(Request { mode: mode.unwrap_or(Mode::Region), delay, output }));
    }
    match mode {
        None if copy || save.is_some() || delay > 0 => Err("--copy, --save and --delay need --region, --last-region, --scroll or --screen".into()),
        None => Ok(Command::Open),
        Some(mode) => {
            let output = if copy || save.is_some() { Output::Direct { copy, save } } else { Output::Editor };
            Ok(Command::Capture(Request { mode, delay, output }))
        }
    }
}

/// Returns the exit status to give now. A capture sets its own when it ends.
fn handle_args(app: &adw::Application, backend: &Rc<dyn CaptureBackend>, cmd: &gio::ApplicationCommandLine, args: &[String]) -> i32 {
    match parse_args(args) {
        Err(msg) => {
            cmd.printerr_literal(&format!("obtura: {msg}. Try --help.\n"));
            2
        }
        Ok(Command::Help) => {
            cmd.print_literal(USAGE);
            0
        }
        Ok(Command::Version) => {
            cmd.print_literal(&format!("obtura {}\n", env!("CARGO_PKG_VERSION")));
            0
        }
        Ok(Command::Library) => {
            crate::library::open_window(app);
            0
        }
        Ok(Command::Open) => {
            open(app);
            0
        }
        Ok(Command::Record(what)) => {
            let cmd = cmd.clone();
            recorder::toggle(
                app,
                backend.clone(),
                what,
                Box::new(move |outcome| {
                    match &outcome {
                        Outcome::Failed(why) => cmd.printerr_literal(&format!("obtura: {why}\n")),
                        Outcome::Printed(text) => cmd.print_literal(&format!("{text}\n")),
                        _ => {}
                    }
                    STATUS.set(Some(outcome.exit_code()));
                    cmd.set_exit_status(outcome.exit_code());
                }),
            );
            0
        }
        Ok(Command::Capture(request)) => {
            // Keeping the command line alive keeps the caller waiting for the real status.
            let cmd = cmd.clone();
            capture::start(
                app,
                backend.clone(),
                request,
                Box::new(move |outcome| {
                    match &outcome {
                        Outcome::Failed(why) => cmd.printerr_literal(&format!("obtura: {why}\n")),
                        Outcome::Printed(text) => cmd.print_literal(&format!("{text}\n")),
                        _ => {}
                    }
                    STATUS.set(Some(outcome.exit_code()));
                    cmd.set_exit_status(outcome.exit_code()); // for a caller forwarded to a running instance
                }),
            );
            0
        }
    }
}

/// Obtura opened without a task: while recording this pauses the recording and shows what can
/// be done; otherwise its window comes forward (the start window if there is none).
fn open(app: &adw::Application) {
    if recorder::is_recording() {
        obtura_core::log::info("opened while recording: pausing");
        return recorder::pause_and_show();
    }
    if crate::scroll::is_running() {
        obtura_core::log::info("opened during a scrolling capture: finishing it");
        return crate::scroll::finish();
    }
    // A window that is on the screen comes forward (a see-through one is in a capture).
    let shown = |w: &gtk::Window| w.is_visible() && w.opacity() > 0.0;
    match app.active_window().filter(shown).or_else(|| app.windows().into_iter().find(shown)) {
        Some(w) => {
            obtura_core::log::info(&format!("opened: bringing “{}” forward", w.title().unwrap_or_default()));
            w.present()
        }
        None => {
            obtura_core::log::info("opened: showing the start window");
            home::open(app);
        }
    }
}

fn install_actions(app: &adw::Application, backend: &Rc<dyn CaptureBackend>) {
    let captures = [
        ("capture-region", Request::interactive(Mode::Region)),
        ("capture-screen", Request::interactive(Mode::Screen)),
        ("capture-last-region", Request::interactive(Mode::LastRegion)),
        ("capture-scrolling", Request::interactive(Mode::Scrolling)),
        ("capture-window", Request::interactive(Mode::Window)),
        ("capture-text", Request { mode: Mode::Region, delay: 0, output: Output::Text }),
        ("capture-qr", Request { mode: Mode::Region, delay: 0, output: Output::Qr }),
        ("capture-pin", Request { mode: Mode::Region, delay: 0, output: Output::Pin }),
        ("pick-color", Request { mode: Mode::Screen, delay: 0, output: Output::Color }),
        ("measure-screen", Request { mode: Mode::Screen, delay: 0, output: Output::Measure }),
    ];
    for (name, request) in captures {
        let action = gio::SimpleAction::new(name, None);
        let backend = backend.clone();
        action.connect_activate({
            let app = app.clone();
            move |_, _| capture::start(&app, backend.clone(), request.clone(), Box::new(|_| {}))
        });
        app.add_action(&action);
    }
    // The same captures after a delay in seconds (the editor's New menu), counted down in the
    // top bar.
    for (name, mode) in [("capture-region-in", Mode::Region), ("capture-screen-in", Mode::Screen)] {
        let action = gio::SimpleAction::new(name, Some(glib::VariantTy::INT32));
        let (app2, backend) = (app.clone(), backend.clone());
        action.connect_activate(move |_, p| {
            let delay = p.and_then(|v| v.get::<i32>()).unwrap_or(0).clamp(0, 30) as u32;
            capture::start(&app2, backend.clone(), Request { mode, delay, output: Output::Editor }, Box::new(|_| {}))
        });
        app.add_action(&action);
    }
    let record = gio::SimpleAction::new("record", None);
    record.connect_activate({
        let (app, backend) = (app.clone(), backend.clone());
        move |_, _| recorder::setup(&app, backend.clone())
    });
    app.add_action(&record);
    for (name, what) in [("record-area", recorder::What::Area), ("record-screen", recorder::What::Screen), ("record-window", recorder::What::Window), ("record-all-screens", recorder::What::AllScreens)] {
        let action = gio::SimpleAction::new(name, None);
        let (app2, backend) = (app.clone(), backend.clone());
        action.connect_activate(move |_, _| recorder::toggle(&app2, backend.clone(), what, Box::new(|_| {})));
        app.add_action(&action);
    }
    // A link read from a QR code, opened from the notification's button by the user.
    let open_link = gio::SimpleAction::new("open-link", Some(glib::VariantTy::STRING));
    open_link.connect_activate(|_, link| {
        let Some(link) = link.and_then(|l| l.get::<String>()) else { return };
        let code = obtura_image::QrCode { text: link.clone(), corners: [(0.0, 0.0); 4] };
        if code.is_link() {
            gtk::UriLauncher::new(&link).launch(gtk::Window::NONE, gio::Cancellable::NONE, |_| {});
        }
    });
    app.add_action(&open_link);
    let scroll_finish = gio::SimpleAction::new("scroll-finish", None);
    scroll_finish.connect_activate(|_, _| crate::scroll::finish());
    app.add_action(&scroll_finish);
    let stop = gio::SimpleAction::new("record-stop", None);
    stop.connect_activate(|_, _| recorder::stop());
    app.add_action(&stop);
    let controls = gio::SimpleAction::new("record-controls", None);
    controls.connect_activate(|_, _| recorder::pause_and_show());
    app.add_action(&controls);
    app.set_accels_for_action("app.record", &["<Ctrl><Shift>E"]);
    let prefs = gio::SimpleAction::new("preferences", None);
    prefs.connect_activate({
        let app = app.clone();
        move |_, _| preferences::show(&app)
    });
    app.add_action(&prefs);
    crate::help::install(app);
    let library = gio::SimpleAction::new("library", None);
    library.connect_activate({
        let app = app.clone();
        move |_, _| crate::library::open_window(&app)
    });
    app.add_action(&library);
    app.set_accels_for_action("app.library", &["<Ctrl>L"]);
    let keys = gio::SimpleAction::new("shortcuts", None);
    keys.connect_activate({
        let app = app.clone();
        move |_, _| preferences::shortcuts(&app)
    });
    app.add_action(&keys);
    // Ctrl+? needs Shift as well on most keyboards (on a French one, Ctrl and the ? key alone is
    // Ctrl+, : Preferences), so Ctrl+F1 opens the list too.
    app.set_accels_for_action("app.shortcuts", &["<Ctrl>question", "<Ctrl>F1"]);
    app.set_accels_for_action("app.capture-region", &["<Ctrl><Shift>R"]);
    app.set_accels_for_action("app.capture-screen", &["<Ctrl><Shift>F"]);
    app.set_accels_for_action("app.capture-text", &["<Ctrl><Shift>T"]);
    app.set_accels_for_action("app.preferences", &["<Ctrl>comma"]);
    app.set_accels_for_action("win.close", &["<Ctrl>W"]);
}

/// The fake backend when `OBTURA_FAKE_CAPTURE` names a PNG (tests and `scripts/smoke.sh`;
/// with `OBTURA_FAKE_PERMISSION=ask` its first screenshot is refused as GNOME does before the
/// user was asked), the portals otherwise.
fn backend() -> Rc<dyn CaptureBackend> {
    if let Some(path) = std::env::var_os("OBTURA_FAKE_CAPTURE").map(PathBuf::from) {
        // OBTURA_FAKE_SCROLL names a tall PNG the fake screen scrolls through (scrolling capture).
        let page = std::env::var_os("OBTURA_FAKE_SCROLL").and_then(|p| std::fs::read(p).ok());
        let scrolled = |b: FakeBackend| match &page {
            Some(png) => b.scrolling(png).unwrap_or_else(|e| {
                eprintln!("obtura: OBTURA_FAKE_SCROLL: {e}");
                FakeBackend::failing(e)
            }),
            None => b,
        };
        match FakeBackend::from_file(&path).map(scrolled) {
            Ok(b) if std::env::var("OBTURA_FAKE_PERMISSION").as_deref() == Ok("ask") => return Rc::new(b.asking_permission()),
            Ok(b) => return Rc::new(b),
            Err(e) => eprintln!("obtura: OBTURA_FAKE_CAPTURE: {e}"),
        }
    }
    Rc::new(PortalBackend::new(APP_ID))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Command, String> {
        parse_args(&s.split_whitespace().map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn no_options_opens_the_start_window() {
        assert_eq!(parse(""), Ok(Command::Open));
    }

    #[test]
    fn region_and_screen_open_the_editor() {
        assert_eq!(parse("--region"), Ok(Command::Capture(Request { mode: Mode::Region, delay: 0, output: Output::Editor })));
        assert_eq!(parse("--screen --delay 5"), Ok(Command::Capture(Request { mode: Mode::Screen, delay: 5, output: Output::Editor })));
        assert_eq!(parse("--last-region --copy"), Ok(Command::Capture(Request { mode: Mode::LastRegion, delay: 0, output: Output::Direct { copy: true, save: None } })));
    }

    #[test]
    fn copy_and_save_skip_the_editor() {
        assert_eq!(parse("--region --copy"), Ok(Command::Capture(Request { mode: Mode::Region, delay: 0, output: Output::Direct { copy: true, save: None } })));
        assert_eq!(parse("--screen --save /tmp/a.png"), Ok(Command::Capture(Request { mode: Mode::Screen, delay: 0, output: Output::Direct { copy: false, save: Some("/tmp/a.png".into()) } })));
        // No path after --save: the save folder is used (an empty path means that).
        assert_eq!(parse("--screen --save --copy"), Ok(Command::Capture(Request { mode: Mode::Screen, delay: 0, output: Output::Direct { copy: true, save: Some(PathBuf::new()) } })));
    }

    #[test]
    fn wrong_options_are_refused_with_a_reason() {
        assert!(parse("--bogus").unwrap_err().contains("--bogus"));
        assert!(parse("--copy").unwrap_err().contains("--region, --last-region, --scroll or --screen"));
        assert!(parse("--screen --delay 31").is_err());
        assert!(parse("--screen --delay x").is_err());
        assert!(parse("--screen --delay").is_err());
    }

    #[test]
    fn text_pin_and_colour_take_a_region_by_default() {
        assert_eq!(parse("--ocr"), Ok(Command::Capture(Request { mode: Mode::Region, delay: 0, output: Output::Text })));
        assert_eq!(parse("--screen --ocr"), Ok(Command::Capture(Request { mode: Mode::Screen, delay: 0, output: Output::Text })));
        assert_eq!(parse("--pin --delay 3"), Ok(Command::Capture(Request { mode: Mode::Region, delay: 3, output: Output::Pin })));
        assert_eq!(parse("--pick-color"), Ok(Command::Capture(Request { mode: Mode::Screen, delay: 0, output: Output::Color })));
        assert_eq!(parse("--qr"), Ok(Command::Capture(Request { mode: Mode::Region, delay: 0, output: Output::Qr })));
        assert_eq!(parse("--qr --last-region"), Ok(Command::Capture(Request { mode: Mode::LastRegion, delay: 0, output: Output::Qr })));
        assert!(parse("--qr --ocr").is_err());
        assert!(parse("--qr --record").is_err());
        assert!(parse("--ocr --pin").is_err());
        assert!(parse("--ocr --copy").is_err());
        assert!(parse("--region --pick-color").is_err());
        assert_eq!(parse("--measure"), Ok(Command::Capture(Request { mode: Mode::Screen, delay: 0, output: Output::Measure })));
        assert!(parse("--measure --region").is_err());
        assert_eq!(parse("--scroll --save /tmp/x.png"), Ok(Command::Capture(Request { mode: Mode::Scrolling, delay: 0, output: Output::Direct { copy: false, save: Some("/tmp/x.png".into()) } })));
        assert_eq!(parse("--scroll"), Ok(Command::Capture(Request { mode: Mode::Scrolling, delay: 0, output: Output::Editor })));
        assert!(parse("--measure --qr").is_err());
    }

    #[test]
    fn record_takes_an_area_or_a_screen() {
        assert_eq!(parse("--record"), Ok(Command::Record(recorder::What::Area)));
        assert_eq!(parse("--record --region"), Ok(Command::Record(recorder::What::Area)));
        assert_eq!(parse("--screen --record"), Ok(Command::Record(recorder::What::Screen)));
        assert!(parse("--record --copy").is_err());
        assert!(parse("--record --delay 3").is_err());
        assert!(parse("--record --ocr").is_err());
        assert!(parse("--record --last-region").is_err());
        assert_eq!(parse("--record --window"), Ok(Command::Record(recorder::What::Window)));
        assert_eq!(parse("--all-screens --record"), Ok(Command::Record(recorder::What::AllScreens)));
        assert!(parse("--record --window --screen").is_err());
        assert_eq!(parse("--window"), Ok(Command::Capture(Request { mode: Mode::Window, delay: 0, output: Output::Editor })));
        assert_eq!(parse("--window --copy"), Ok(Command::Capture(Request { mode: Mode::Window, delay: 0, output: Output::Direct { copy: true, save: None } })));
        assert!(parse("--window --region").is_err());
        assert!(parse("--all-screens").is_err());
    }

    /// GNOME runs the desktop file's actions (the dock icon's menu) as the app actions of the
    /// same names, since Obtura is D-Bus activatable: a name without an action does nothing.
    #[test]
    fn every_desktop_action_is_an_app_action() {
        const DESKTOP: &str = include_str!("../../data/com.grasshillsco.Obtura.desktop");
        const SOURCE: &str = include_str!("app.rs");
        let actions = DESKTOP.lines().find_map(|l| l.strip_prefix("Actions=")).expect("an Actions line");
        let names: Vec<&str> = actions.split(';').filter(|a| !a.is_empty()).collect();
        assert!(names.len() >= 2, "{actions}");
        for a in names {
            assert!(SOURCE.contains(&format!("(\"{a}\", ")) || SOURCE.contains(&format!("SimpleAction::new(\"{a}\"")), "no app action named {a}");
            assert!(DESKTOP.contains(&format!("[Desktop Action {a}]")), "no [Desktop Action {a}] group");
        }
    }

    #[test]
    fn the_shortcuts_added_to_gnome_run_real_commands() {
        for k in &crate::global_keys::KEYS {
            let args: Vec<String> = k.command.split_whitespace().skip(1).map(String::from).collect();
            assert!(matches!(parse_args(&args), Ok(Command::Capture(_) | Command::Record(_))), "{}", k.command);
        }
    }

    #[test]
    fn help_and_version_win_over_everything() {
        assert_eq!(parse("--region --help"), Ok(Command::Help));
        assert_eq!(parse("--version"), Ok(Command::Version));
    }
}
