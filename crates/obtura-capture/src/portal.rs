//! The real backend: the `org.freedesktop.portal.Screenshot` portal, called directly over
//! the session bus with GIO (no `ashpd`). GIO is GLib, not GTK.
//!
//! The portal always writes the picture to a file in the user's Pictures folder
//! (`Screenshot.png`) and returns its address. We read it into memory and remove that one
//! file at once, so nothing of ours is left in the user's folder. See [`consume_portal_file`]
//! for the exact rule.
//!
//! Permission: the portal lets an app take screenshots without asking each time only once the
//! user allowed it ("Allow Obtura to Take Screenshots?", kept in the system's permission store
//! under the app's ID). GTK 4.22 tells the portal Obtura's ID once Obtura is installed, so the
//! permission is Obtura's own, and GNOME shows that question only while one of Obtura's windows
//! has the focus: from a hidden Obtura it refuses (answer 2) without asking. So when the portal
//! refuses, the permission store says why: no answer yet ([`CaptureError::NeedsPermission`]:
//! ask from a focused window), "no" ([`CaptureError::Denied`]), or "yes" (a real failure).

use crate::{CaptureBackend, CaptureError, CaptureKind, CaptureRequest, Done, Shot};
use gio::prelude::*;
use gio::glib::{variant::ToVariant, Variant, VariantDict};
use std::{cell::RefCell, path::Path, rc::Rc, time::Duration};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENSHOT: &str = "org.freedesktop.portal.Screenshot";
const REQUEST: &str = "org.freedesktop.portal.Request";
const STORE_DEST: &str = "org.freedesktop.impl.portal.PermissionStore";
const STORE_PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
/// The permission store's table and entry for screenshots (as the portal names them).
const TABLE: &str = "screenshot";
const ENTRY: &str = "screenshot";

/// A file the portal made less than this long ago is the one we asked for.
const FRESH: Duration = Duration::from_secs(60);

pub struct PortalBackend {
    /// The application ID the portal knows Obtura by (its permissions are kept under it).
    app_id: String,
}

impl PortalBackend {
    pub fn new(app_id: &str) -> Self {
        Self { app_id: app_id.to_string() }
    }
}

impl CaptureBackend for PortalBackend {
    fn capture(&self, request: CaptureRequest, done: Done) {
        // Region and monitor shots are cut from the full-screen picture by the caller.
        let CaptureKind::FullScreen = request.kind;
        let conn = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
            Ok(c) => c,
            Err(e) => return done(Err(CaptureError::Failed(format!("no session bus: {e}")))),
        };
        screenshot(conn, self.app_id.clone(), done);
    }

    fn screencast(&self, request: crate::CastRequest, done: crate::CastDone) {
        crate::screencast::start(request, done);
    }

    /// Removes Obtura's own "no" from the permission store (only that entry), so the portal asks
    /// the user again. Nothing is granted here.
    fn ask_again(&self, done: Box<dyn FnOnce()>) {
        let conn = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
            Ok(c) => c,
            Err(_) => return done(),
        };
        let args = (TABLE, ENTRY, self.app_id.as_str()).to_variant();
        conn.call(Some(STORE_DEST), STORE_PATH, STORE_DEST, "DeletePermission", Some(&args), None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE, move |_| done());
    }
}

fn screenshot(conn: gio::DBusConnection, app_id: String, done: Done) {
    let sender = conn.unique_name().map(|n| n.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    let token = format!("obtura_{}_{}", std::process::id(), next_token());
    let request_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");

    // Subscribe BEFORE calling, or a fast answer is missed.
    let done = Rc::new(RefCell::new(Some(done)));
    // Dropping the subscription unsubscribes.
    let sub: Rc<RefCell<Option<gio::SignalSubscription>>> = Rc::new(RefCell::new(None));
    let (done2, sub2) = (done.clone(), sub.clone());
    let id = conn.subscribe_to_signal(
        Some(DEST),
        Some(REQUEST),
        Some("Response"),
        Some(&request_path),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            sub2.borrow_mut().take();
            let code: u32 = signal.parameters.child_value(0).get().unwrap_or(99);
            let results = VariantDict::new(Some(&signal.parameters.child_value(1)));
            let uri = results.lookup_value("uri", None).and_then(|v| v.str().map(str::to_owned));
            let Some(done) = done2.borrow_mut().take() else { return };
            if code != 2 {
                return done(shot_from_response(code, uri.as_deref()));
            }
            // Refused: the permission store says whether the user was asked, and what they said.
            let conn = signal.connection.clone();
            let args = (TABLE, ENTRY).to_variant();
            let app_id = app_id.clone();
            conn.call(Some(STORE_DEST), STORE_PATH, STORE_DEST, "Lookup", Some(&args), None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE, move |reply| {
                // No table yet (nobody was ever asked) is an error from the store: no answer.
                let answer = reply.ok().and_then(|v| permission_of(&v.child_value(0), &app_id));
                done(Err(refusal(answer.as_deref())));
            });
        },
    );
    *sub.borrow_mut() = Some(id);

    let options = VariantDict::new(None);
    options.insert_value("handle_token", &token.to_variant());
    options.insert_value("interactive", &false.to_variant());
    let args = Variant::tuple_from_iter([String::new().to_variant(), options.end()]);
    let (done3, sub3) = (done.clone(), sub.clone());
    conn.call(
        Some(DEST),
        PATH,
        SCREENSHOT,
        "Screenshot",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
        move |reply| {
            let Err(e) = reply else { return };
            // The call failed, so no Response will come.
            sub3.borrow_mut().take();
            if let Some(done) = done3.borrow_mut().take() {
                let denied = e.matches(gio::DBusError::AccessDenied);
                done(Err(if denied { CaptureError::Denied } else { CaptureError::Failed(e.message().to_string()) }));
            }
        },
    );
}

fn next_token() -> u32 {
    thread_local!(static N: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });
    N.with(|n| {
        n.set(n.get() + 1);
        n.get()
    })
}

/// What the permission store holds for `app_id` in a table of `a{sas}` (app → permissions).
fn permission_of(table: &Variant, app_id: &str) -> Option<Vec<String>> {
    table.iter().find_map(|entry| {
        let app: String = entry.child_value(0).get()?;
        (app == app_id).then(|| entry.child_value(1).get::<Vec<String>>()).flatten()
    })
}

/// Why the portal refused a screenshot, from what the permission store holds for Obtura.
pub(crate) fn refusal(answer: Option<&[String]>) -> CaptureError {
    match answer {
        Some(a) if a.iter().any(|p| p == "yes") => CaptureError::Failed("the system could not take the screenshot".into()),
        Some(a) if a.iter().any(|p| p == "no") => CaptureError::Denied,
        _ => CaptureError::NeedsPermission,
    }
}

/// Turns the portal's `Response` (code and file address) into a [`Shot`].
/// Code 0 is success, 1 is "the user cancelled", anything else is a failure.
pub(crate) fn shot_from_response(code: u32, uri: Option<&str>) -> Result<Shot, CaptureError> {
    match code {
        0 => {}
        1 => return Err(CaptureError::Cancelled),
        _ => return Err(CaptureError::Failed("the system could not take the screenshot".into())),
    }
    let uri = uri.ok_or_else(|| CaptureError::Failed("the system did not say where the picture is".into()))?;
    let path = gio::File::for_uri(uri)
        .path()
        .ok_or_else(|| CaptureError::Failed("the picture is not a local file".into()))?;
    Shot::from_png(consume_portal_file(&path)?)
}

/// Reads the picture the portal made, then removes that exact file, but only if it is
/// clearly the portal's: a regular file (not a link) called `Screenshot*.png`, made in
/// the last minute. Anything else stays where it is: a picture of the owner's is never
/// deleted. (The portal may overwrite an owner's own `Screenshot.png`; that is the
/// portal's behaviour and cannot be avoided.)
pub(crate) fn consume_portal_file(path: &Path) -> Result<Vec<u8>, CaptureError> {
    let failed = |e: std::io::Error| CaptureError::Failed(format!("{}: {e}", path.display()));
    let meta = std::fs::symlink_metadata(path).map_err(failed)?;
    let bytes = std::fs::read(path).map_err(failed)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let ours = meta.is_file()
        && name.starts_with("Screenshot")
        && name.ends_with(".png")
        && meta.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < FRESH);
    if ours {
        let _ = std::fs::remove_file(path);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/sample.png");

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("obtura-capture-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn removes_the_fresh_portal_file_after_reading_it() {
        let dir = scratch("fresh");
        let file = dir.join("Screenshot.png");
        std::fs::write(&file, FIXTURE).unwrap();
        assert_eq!(consume_portal_file(&file).unwrap(), FIXTURE);
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_an_old_file_even_if_named_like_the_portals() {
        let dir = scratch("old");
        let file = dir.join("Screenshot.png");
        std::fs::write(&file, FIXTURE).unwrap();
        let old = SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(old).unwrap();
        assert_eq!(consume_portal_file(&file).unwrap(), FIXTURE);
        assert!(file.exists(), "an owner's older picture must never be deleted");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_a_fresh_file_with_another_name() {
        let dir = scratch("name");
        let file = dir.join("holiday.png");
        std::fs::write(&file, FIXTURE).unwrap();
        consume_portal_file(&file).unwrap();
        assert!(file.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn never_deletes_through_a_link() {
        let dir = scratch("link");
        let target = dir.join("precious.png");
        std::fs::write(&target, FIXTURE).unwrap();
        let link = dir.join("Screenshot.png");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        consume_portal_file(&link).unwrap();
        assert!(target.exists() && link.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_file_is_an_error() {
        assert!(consume_portal_file(Path::new("/nonexistent/Screenshot.png")).is_err());
    }

    #[test]
    fn a_refusal_is_explained_by_the_permission_store() {
        let yes = vec!["yes".to_string()];
        let no = vec!["no".to_string()];
        assert_eq!(refusal(None), CaptureError::NeedsPermission);
        assert_eq!(refusal(Some(&[])), CaptureError::NeedsPermission);
        assert_eq!(refusal(Some(&no)), CaptureError::Denied);
        assert!(matches!(refusal(Some(&yes)), CaptureError::Failed(_)));
    }

    #[test]
    fn reads_the_entry_of_this_app_only() {
        // As the store answers Lookup("screenshot", "screenshot"): every app's answer.
        let table = Variant::parse(Some(gio::glib::VariantTy::new("a{sas}").unwrap()), "{'': ['yes'], 'com.example.Other': ['no']}").unwrap();
        assert_eq!(permission_of(&table, "com.grasshillsco.Obtura"), None);
        assert_eq!(permission_of(&table, ""), Some(vec!["yes".to_string()]));
        assert_eq!(permission_of(&table, "com.example.Other"), Some(vec!["no".to_string()]));
    }

    #[test]
    fn maps_portal_response_codes() {
        assert_eq!(shot_from_response(1, None).unwrap_err(), CaptureError::Cancelled);
        assert!(matches!(shot_from_response(2, None), Err(CaptureError::Failed(_))));
        assert!(matches!(shot_from_response(0, None), Err(CaptureError::Failed(_))));
        let dir = scratch("resp");
        let file = dir.join("Screenshot.png");
        std::fs::write(&file, FIXTURE).unwrap();
        let shot = shot_from_response(0, Some(&format!("file://{}", file.display()))).unwrap();
        assert_eq!((shot.width, shot.height), (800, 500));
        let _ = std::fs::remove_dir_all(dir);
    }
}
