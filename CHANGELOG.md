# Changelog

## 0.9.1 (2026-11-23)

- **Windows version** (Windows 10 and 11, 64-bit): an installer (for the current user, no
  administrator rights) and a zip to run without installing, cross-compiled with GTK and
  libadwaita from MSYS2. Screenshots are read from the screen directly (no question, no
  flash); Print Screen, Shift+Print Screen and Ctrl+Print Screen work in every app;
  Obtura's icon in the notification area offers the captures; "A Window" takes the window
  in front; scrolling capture, text reading (Tesseract included, English), QR codes, the
  editor, pins (kept above other windows), the library. Screen recording is not in the
  Windows version yet.
- Windows: Obtura draws with the processor (GTK's Cairo renderer) instead of OpenGL: it
  stopped at start inside an NVIDIA OpenGL driver.
- Windows: in the Formatting and Beautify panels, the colour chooser picked a colour only
  on the second click (the first one closed the panel and left the chooser alone on the
  screen). The panel now closes while the chooser is open and comes back after; the same
  for the font chooser and Beautify's background picture.
- Editor: a mark below a blur, pixelate, box or magnifier kept its old look on the screen
  when it was resized by its handles, turned or bent, until something else changed (the
  saved picture was right). The view now follows at once.

## 0.9.0 (2026-11-18)

The first package: the release candidate for 1.0.

- One package for Ubuntu 24.04 LTS and every newer release, built and tested on
  Ubuntu 24.04, install-tested on 24.04 and 26.04.
- **Capture**: a region (on a still picture of the screen, without GNOME's flash), the
  whole screen, the same region again, after a delay (counted down in the top bar). The
  picture is on the clipboard at once and the editor opens; its New button takes the same
  kind of capture again (region, scrolling, full screen...), as chosen beside it.
- **Scrolling capture**: select what scrolls, scroll it, and get the whole page in one
  picture (a sticky header kept once); `--scroll`.
- **Editor**: lines and arrows, curves, rectangles, ellipses, pen, highlighter, text,
  callouts, numbered steps, emoji and symbols, spotlight, magnifier, pasted pictures and
  pasted text; every mark stays editable (handles, a round handle to turn it, alignment
  guides, Formatting panel with any installed font, styles saved per tool); curves through
  as many bends as wanted. Zoom, undo and redo without limit. Marks past the edge make the
  picture grow.
- **Redaction that is real**: blur, pixelate, box (square or round); Smart Redact finds
  email addresses, phone and card numbers, IBANs, IP addresses, keys, passwords and QR
  codes.
- **Beautify**: backgrounds (colours, gradients, a picture of your own), padding, rounded
  corners, shadow, border, window bar, phone, tablet, laptop and monitor frames, torn
  edge, size ratios; your own frame, added to every capture if you like.
- **Image menu**: rotate, flip, resize, add space, trim plain borders, attach or place
  another picture, combine several pictures in a row, a column or a grid, paste a picture;
  crop with ratios; cut out a strip.
- **Text, codes and colour**: Copy Text from a picture or straight from the screen
  (Tesseract), Read QR Codes (in the editor, from the screen, `--qr`), colour picker in
  the editor and on the screen (hex, rgb() or hsl()), measure tool, measuring on the
  screen (`--measure`).
- **Pin** a picture on the screen; **library** of every capture, searchable by its text
  and by tags, accents and capitals ignored.
- **Recording**: a screen, an area, a window or all screens at once as MP4, WebM or GIF,
  microphone and computer sound with level meters, pointer, countdown; pause, resume and
  stop from Obtura's icon in the top bar; trim afterwards; small optimised GIFs.
- **Keys in every app**: one click adds Super+Shift+S (region), Super+Shift+T (text) and
  Super+Shift+R (record) to GNOME's keyboard shortcuts.
- **Command line**: `--region`, `--screen`, `--last-region`, `--scroll`, `--delay`,
  `--copy`, `--save`, `--ocr`, `--qr`, `--pin`, `--pick-color`, `--measure`, `--record`
  (with `--screen`, `--window`, `--all-screens`), `--library`, with exit codes.
- **Help**: the user guide (F1), Report a Problem, About with What's New and the licences
  of the software Obtura contains; `man obtura`.
- **Log file** `~/.local/state/obtura/obtura.log` (errors, warnings, crashes; no pictures or
  text; Show Log File in the main menu).
