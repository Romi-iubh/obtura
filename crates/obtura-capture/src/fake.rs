use crate::{CaptureBackend, CaptureError, CaptureRequest, Done, Shot};
use std::{cell::Cell, path::Path};

/// Returns a fixed picture (or a fixed error) at once, so everything above the capture
/// trait can be tested without a desktop. Used by the tests and by `scripts/smoke.sh`.
pub struct FakeBackend {
    result: Result<Shot, CaptureError>,
    /// A tall page the shared screen scrolls through (scrolling capture tests).
    page: Option<obtura_image::Bitmap>,
    /// Captures still to refuse with [`CaptureError::NeedsPermission`], as GNOME does before the
    /// user was asked (see [`FakeBackend::asking_permission`]).
    refusals: Cell<u32>,
    /// Windows on the fake screen, as Windows lists them (see [`FakeBackend::with_windows`]).
    windows: Option<Vec<crate::Place>>,
}

impl FakeBackend {
    pub fn from_png(png: Vec<u8>) -> Result<Self, CaptureError> {
        Ok(Self { result: Ok(Shot::from_png(png)?), page: None, refusals: Cell::new(0), windows: None })
    }

    /// The first screenshot is refused because the user was never asked, as on a fresh GNOME;
    /// the next one (made from a focused window, where GNOME asks) works.
    pub fn asking_permission(self) -> Self {
        self.refusals.set(1);
        self
    }

    pub fn from_file(path: &Path) -> Result<Self, CaptureError> {
        let png = std::fs::read(path).map_err(|e| CaptureError::Failed(format!("{}: {e}", path.display())))?;
        Self::from_png(png)
    }

    /// The shared screen shows `page` (a tall PNG) and scrolls through it while a scrolling
    /// capture runs.
    pub fn scrolling(mut self, page_png: &[u8]) -> Result<Self, CaptureError> {
        self.page = Some(obtura_image::decode_png(page_png).map_err(|e| CaptureError::Failed(e.to_string()))?);
        Ok(self)
    }

    /// The fake screen holds these windows (front first), listed as Windows lists them, so the
    /// window is picked on Obtura's own overlay (as on Windows) rather than in GNOME's list.
    pub fn with_windows(mut self, windows: Vec<crate::Place>) -> Self {
        self.windows = Some(windows);
        self
    }

    /// A backend whose every capture fails with `error` (for testing the error paths).
    pub fn failing(error: CaptureError) -> Self {
        Self { result: Err(error), page: None, refusals: Cell::new(0), windows: None }
    }
}

impl CaptureBackend for FakeBackend {
    fn shares_real_screen(&self) -> bool {
        false
    }

    fn windows(&self) -> Option<Vec<crate::Place>> {
        self.windows.clone()
    }

    fn capture(&self, _request: CaptureRequest, done: Done) {
        if self.refusals.get() > 0 {
            self.refusals.set(self.refusals.get() - 1);
            return done(Err(CaptureError::NeedsPermission));
        }
        done(self.result.clone());
    }

    /// A moving test picture as big as the fixture (the virtual screen in the smoke tests); for
    /// a window, a smaller test picture (as if a window had been picked in GNOME's list).
    fn screencast(&self, request: crate::CastRequest, done: crate::CastDone) {
        done(match &self.result {
            Ok(_) if request.windows => Ok(crate::Cast::fake(480, 320, None)),
            Ok(shot) => {
                let cast = crate::Cast::fake(shot.width, shot.height, Some(shot.png.clone()));
                Ok(match &self.page {
                    Some(page) => cast.with_page(page.clone()),
                    None => cast,
                })
            }
            Err(e) => Err(e.clone()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CaptureKind;
    use std::{cell::RefCell, rc::Rc};

    const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/sample.png");

    fn grab(backend: &FakeBackend) -> Result<Shot, CaptureError> {
        let slot = Rc::new(RefCell::new(None));
        let s = slot.clone();
        backend.capture(CaptureRequest { kind: CaptureKind::FullScreen }, Box::new(move |r| *s.borrow_mut() = Some(r)));
        slot.take().expect("the fake backend answers at once")
    }

    #[test]
    fn returns_the_fixture_with_its_size() {
        let shot = grab(&FakeBackend::from_png(FIXTURE.to_vec()).unwrap()).unwrap();
        assert_eq!((shot.width, shot.height), (800, 500));
        assert_eq!(shot.png, FIXTURE);
    }

    #[test]
    fn refuses_bytes_that_are_not_a_png() {
        assert!(FakeBackend::from_png(b"not a picture at all, just text".to_vec()).is_err());
        assert!(FakeBackend::from_png(Vec::new()).is_err());
    }

    #[test]
    fn asks_for_permission_once() {
        let backend = FakeBackend::from_png(FIXTURE.to_vec()).unwrap().asking_permission();
        assert_eq!(grab(&backend).unwrap_err(), CaptureError::NeedsPermission);
        assert!(grab(&backend).is_ok());
        assert!(grab(&backend).is_ok());
    }

    #[test]
    fn passes_errors_through() {
        let backend = FakeBackend::failing(CaptureError::Cancelled);
        assert_eq!(grab(&backend).unwrap_err(), CaptureError::Cancelled);
    }
}
