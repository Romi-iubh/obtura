#!/usr/bin/env bash
# Headless launch under a virtual display with the fake capture backend.
# Usage: scripts/smoke.sh path/to/image.png [screen|region|edit|annotate|phase2|phase3|cli|record]
#   screen (default): starts a full-screen capture, waits 6 s, takes a screenshot, fails
#                     if the app exits early.
#   region: starts a region capture, drags a rectangle with xdotool, accepts it with Enter,
#           and checks that the editor window opens at the size of the rectangle.
#   edit: opens the editor on the picture, draws a rectangle, a numbered step and a box
#         over the text with the real mouse and keys, saves with Ctrl+S, and checks the
#         pixels of the saved file (marks present, covered text really gone).
#   annotate: draws an arrow, a curve (and bends it by its handle), a callout with typed text,
#             a numbered step and a round blur, saves, and checks the pixels of the saved
#             file; then checks that "New" asks before dropping changes and replaces the window.
#   phase2: a stamp, a magnifier, a spotlight, a pasted picture, a strip cut out, the Beautify
#           frame; saves and checks the pixels of the saved file. (Uses sample.png.)
#   phase3: an arrow drawn past the right edge makes the saved picture wider (white room);
#           deleting it gives the screenshot's size back, undo brings the room back. Then, on a
#           picture with an email, phone, card and IP: Smart Redact hides them, Copy Text (all,
#           and the G tool on one line) copies the rest and never what was hidden, the colour
#           picker copies a colour, a Measure line shows its label. (Needs tesseract.)
#   record: records with the fake backend (a moving test picture and test tones, never the real
#           screen or microphone): a whole screen as MP4, stopped by running --record again; an
#           area as WebM with two sounds; a pause (running obtura) and Resume, the two parts
#           joined; the top-bar icon (with a stand-in host): its time, Pause, Resume and Stop
#           from its menu; Trim (sliders moved with the keys); a half-size GIF. Checks length,
#           size and sound of each file.
#   scroll: a scrolling capture with the fake screen scrolling through tests/fixtures/long-page.png
#           (--scroll --save, finished by running --scroll again): the saved picture is exactly
#           the page between the top and bottom of the part chosen; then --measure on the screen.
#   extras: in the editor, a rectangle turned by its round handle, a bend added to a line by a
#           double-click and dragged, text pasted from the clipboard as a text box; then Read QR
#           Codes (Ctrl+Shift+Q) on tests/fixtures/qr-one.png copies the link.
#   cli: runs the command line: --save writes a PNG and exits 0; --copy puts the picture on
#        the clipboard; a wrong option exits 2; an unwritable path exits 1; Escape in the
#        region overlay exits 3; --window saves the fake window; --qr reads the two codes of
#        tests/fixtures/qr-two.png.
# Screenshots go to /tmp/obtura-smoke/.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/obtura-target}"

PNG="${1:?usage: scripts/smoke.sh path/to/image.png [screen|region|edit|annotate|phase2|phase3|cli|record|scroll|extras]}"
MODE="${2:-screen}"
if [ ! -f "$PNG" ]; then
  echo "FAIL: image not found: $PNG" >&2
  exit 1
fi
PNG="$(realpath "$PNG")"
read -r W H <<<"$(identify -format '%w %h' "$PNG")"

OUT=/tmp/obtura-smoke
mkdir -p "$OUT"
rm -f "$OUT"/*.png
cargo build -p obtura --offline

export GTK_A11Y=none
# Throwaway settings and library: never touch the owner's data.
SANDBOX="$OUT/home"
rm -rf "$SANDBOX"
mkdir -p "$SANDBOX"
export HOME="$SANDBOX" XDG_CONFIG_HOME="$SANDBOX/config" XDG_DATA_HOME="$SANDBOX/data" XDG_STATE_HOME="$SANDBOX/state" XDG_CACHE_HOME="$SANDBOX/cache"
# GNOME settings (the keys Obtura can add to GNOME's shortcuts) stay in memory: never the desktop's.
export GSETTINGS_BACKEND=memory
# Force the virtual X display; otherwise GTK opens on the real Wayland session.
export GDK_BACKEND=x11
unset WAYLAND_DISPLAY
# The fake backend returns this PNG instead of calling a portal.
export OBTURA_FAKE_CAPTURE="$PNG"
export SMOKE_OUT="$OUT" SMOKE_MODE="$MODE" PNG
# The virtual screen is as big as the picture, so one logical pixel is one picture pixel.
export W H
SCREEN="${W}x${H}"; case "$MODE" in edit|annotate|phase2|phase3|extras) SCREEN="1280x800";; esac
xvfb-run -a -s "-screen 0 ${SCREEN}x24" dbus-run-session -- bash -c '
  shot() { /usr/bin/import -window root "$SMOKE_OUT/$1.png"; }
  if [ "$SMOKE_MODE" = region ]; then
    "$CARGO_TARGET_DIR/debug/obtura" --region &
    APP=$!
    sleep 3; shot 1-overlay
    X0=$((W/8)); Y0=$((H/8)); X1=$((W/2)); Y1=$((H/2))
    xdotool mousemove $X0 $Y0 mousedown 1 mousemove $(( (X0+X1)/2 )) $(( (Y0+Y1)/2 )) mousemove $X1 $Y1 mouseup 1
    sleep 1; shot 2-selected
    xdotool key Return
    sleep 2; shot 3-editor
    xdotool mousemove 240 180 click 1 key ctrl+s
    sleep 1; shot 4-saved
    SAVED=$(ls "$HOME"/Pictures/Screenshots/Screenshot*.png 2>/dev/null | head -1 || true)
    SAVED_SIZE=$([ -n "$SAVED" ] && identify -format "%wx%h" "$SAVED" || echo none)
    TITLE=$(xdotool search --name "Obtura: " getwindowname 2>/dev/null | head -1 || true)
    WANT="Obtura: $((X1-X0)) × $((Y1-Y0))"
    kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null || true
    if [ "$TITLE" != "$WANT" ]; then echo "FAIL: window title is \"$TITLE\", wanted \"$WANT\"" >&2; exit 1; fi
    if [ "$SAVED_SIZE" != "$((X1-X0))x$((Y1-Y0))" ]; then echo "FAIL: Ctrl+S saved: $SAVED_SIZE" >&2; exit 1; fi
    echo "OK: region editor opened as \"$TITLE\" and Ctrl+S saved a $SAVED_SIZE picture"
  elif [ "$SMOKE_MODE" = cli ]; then
    BIN="$CARGO_TARGET_DIR/debug/obtura"
    FAILED=0
    check() { # check "what" expected actual
      if [ "$2" = "$3" ]; then echo "ok: $1"; else echo "FAIL: $1: wanted $2, got $3" >&2; FAILED=1; fi
    }
    # The first screenshot is refused, as GNOME does before the user was asked whether Obtura may
    # take screenshots (it asks only while an Obtura window has the focus): Obtura asks in a
    # window, Continue (Enter), then the capture goes on and the command ends as usual.
    OBTURA_FAKE_PERMISSION=ask "$BIN" --screen --save "$SMOKE_OUT/asked.png" >/dev/null 2>&1 &
    CLI=$!
    sleep 3; shot permission
    check "the permission question shows in a window" 1 "$(xdotool search --onlyvisible --name "^Obtura\$" 2>/dev/null | wc -l)"
    xdotool mousemove 270 320; xdotool key Return; wait $CLI; check "--save after the permission question exit status" 0 $?
    check "--save after the permission question wrote a picture" "${W}x${H}" "$(identify -format "%wx%h" "$SMOKE_OUT/asked.png" 2>/dev/null || echo none)"
    rm -rf "$XDG_DATA_HOME"/obtura/library
    "$BIN" --screen --save "$SMOKE_OUT/cli.png" >/dev/null 2>&1; check "--save exit status" 0 $?
    check "--save wrote a picture" "${W}x${H}" "$(identify -format "%wx%h" "$SMOKE_OUT/cli.png" 2>/dev/null || echo none)"
    # Obtura ends after --save, but not before the capture is in the library.
    check "--save keeps the capture in the library" 1 "$(ls "$XDG_DATA_HOME"/obtura/library/*/original.png 2>/dev/null | wc -l)"
    T0=$(date +%s); "$BIN" --screen --delay 2 --save "$SMOKE_OUT/delayed.png" >/dev/null 2>&1; T1=$(date +%s)
    check "--delay 2 waited at least 2 seconds" yes "$([ $((T1-T0)) -ge 2 ] && echo yes || echo "no (${T1}-${T0})")"
    "$BIN" --bogus >/dev/null 2>&1; check "unknown option exit status" 2 $?
    "$BIN" --copy >/dev/null 2>&1; check "--copy alone exit status" 2 $?
    "$BIN" --screen --delay 99 >/dev/null 2>&1; check "bad --delay exit status" 2 $?
    "$BIN" --screen --save /proc/obtura-cannot-write-here/x.png >/dev/null 2>&1; check "unwritable path exit status" 1 $?
    "$BIN" --version | grep -q "^obtura " ; check "--version prints" 0 $?
    "$BIN" --screen --save >/dev/null 2>&1; check "--save without a path exit status" 0 $?
    check "--save without a path used the save folder" 1 "$(ls "$HOME"/Pictures/Screenshots/Screenshot*.png 2>/dev/null | wc -l)"
    # --copy: Obtura stays running so the clipboard keeps its picture.
    "$BIN" --screen --copy >/dev/null 2>&1 &
    sleep 4
    check "--copy put a PNG on the clipboard" "${W}x${H}" "$(xclip -selection clipboard -t image/png -o 2>/dev/null | identify -format "%wx%h" - 2>/dev/null || echo none)"
    pkill -x obtura 2>/dev/null || true
    sleep 1
    # Escape in the region overlay cancels: exit status 3.
    "$BIN" --region --save "$SMOKE_OUT/never.png" >/dev/null 2>&1 &
    CLI=$!
    sleep 3; xdotool key Escape; wait $CLI; check "cancelled region exit status" 3 $?
    check "cancelled region saved nothing" none "$([ -f "$SMOKE_OUT/never.png" ] && echo file || echo none)"
    # A region, then the same region again without the overlay: the same picture.
    "$BIN" --region --save "$SMOKE_OUT/r1.png" >/dev/null 2>&1 &
    CLI=$!
    sleep 3; xdotool mousemove 50 40 mousedown 1 mousemove 150 100 mousemove 250 160 mouseup 1; sleep 0.5; xdotool key Return; wait $CLI
    "$BIN" --last-region --save "$SMOKE_OUT/r2.png" >/dev/null 2>&1; check "--last-region exit status" 0 $?
    check "--last-region has the size of the last region" "200x120" "$(identify -format "%wx%h" "$SMOKE_OUT/r2.png" 2>/dev/null || echo none)"
    check "--last-region gives the same picture" 0 "$(compare -metric AE "$SMOKE_OUT/r1.png" "$SMOKE_OUT/r2.png" null: 2>&1 | cut -d" " -f1)"
    # The overlay proposes the region chosen last time: Enter alone takes it again.
    "$BIN" --region --save "$SMOKE_OUT/r3.png" >/dev/null 2>&1 &
    CLI=$!
    sleep 3; shot proposed; xdotool key Return; wait $CLI
    check "Enter on the proposed region takes the last region" "200x120" "$(identify -format "%wx%h" "$SMOKE_OUT/r3.png" 2>/dev/null || echo none)"
    # --ocr reads the text of the picture, copies it and prints it. Like --copy, the first
    # instance stays running to keep the clipboard; the calls below are forwarded to it.
    "$BIN" --screen --ocr >"$SMOKE_OUT/ocr.txt" 2>/dev/null &
    sleep 6
    OUT=$(cat "$SMOKE_OUT/ocr.txt")
    check "--ocr printed the text" yes "$(echo "$OUT" | grep -q "fixture" && echo yes || echo "no: $OUT")"
    check "--ocr copied the text" yes "$(xclip -selection clipboard -o 2>/dev/null | grep -q "fixture" && echo yes || echo no)"
    # --pick-color: the frozen screen; a click copies and prints that colour.
    WANT=$(convert "$PNG" -alpha off -crop 1x1+700+400 +repage -format "#%[hex:p{0,0}]" info: | tr "A-F" "a-f")
    "$BIN" --pick-color >"$SMOKE_OUT/color.txt" 2>/dev/null &
    CLI=$!
    sleep 3; xdotool mousemove 700 400; sleep 0.5; shot pick-color; xdotool click 1; wait $CLI; check "--pick-color exit status" 0 $?
    check "--pick-color printed the colour" "$WANT" "$(cat "$SMOKE_OUT/color.txt")"
    check "--pick-color copied the colour" "$WANT" "$(xclip -selection clipboard -o 2>/dev/null)"
    "$BIN" --pick-color >/dev/null 2>&1 &
    CLI=$!
    sleep 3; xdotool key Escape; wait $CLI; check "cancelled --pick-color exit status" 3 $?
    # --pin opens the picture in a small window of its own.
    "$BIN" --screen --pin >/dev/null 2>&1; check "--pin exit status" 0 $?
    sleep 1.5; check "--pin opened a pinned window" 1 "$(xdotool search --name "^Obtura: pinned picture" 2>/dev/null | wc -l)"
    "$BIN" --library >/dev/null 2>&1; sleep 2; shot library
    check "--library opened the library" 1 "$(xdotool search --name "^Obtura Library" 2>/dev/null | wc -l)"
    check "captures went into the library" yes "$([ "$(ls "$XDG_DATA_HOME"/obtura/library/*/original.png 2>/dev/null | wc -l)" -ge 3 ] && echo yes || echo no)"
    pkill -x obtura 2>/dev/null || true
    sleep 1
    # --window: the fake screen sharing gives a 480 x 320 test window (as if picked in the list of GNOME).
    timeout 30 "$BIN" --window --save "$SMOKE_OUT/window.png" >/dev/null 2>&1; check "--window --save exit status" 0 $?
    check "--window saved the window" "480x320" "$(identify -format "%wx%h" "$SMOKE_OUT/window.png" 2>/dev/null || echo none)"
    # --qr on a picture without a code: nothing copied, so Obtura ends, with exit status 1.
    timeout 30 "$BIN" --screen --qr >/dev/null 2>&1; check "--qr with no code exit status" 1 $?
    # --qr reads the QR codes of the picture (a fixture with two), copies and prints them. Having
    # copied, the first instance stays running for the clipboard: started with &, never waited for.
    OBTURA_FAKE_CAPTURE="$PWD/tests/fixtures/qr-two.png" "$BIN" --screen --qr >"$SMOKE_OUT/qr.txt" 2>/dev/null &
    sleep 4
    check "--qr printed both codes" "first code|WIFI:S:Demo;T:WPA;P:not-a-real-secret;;" "$(paste -sd"|" "$SMOKE_OUT/qr.txt")"
    check "--qr copied the codes" yes "$(xclip -selection clipboard -o 2>/dev/null | grep -q "^first code" && echo yes || echo no)"
    pkill -x obtura 2>/dev/null || true
    sleep 1
    [ "$FAILED" = 0 ] && echo "OK: command line behaves" || { echo "FAIL: see above" >&2; exit 1; }
  elif [ "$SMOKE_MODE" = annotate ]; then
    # The picture is shown 1:1 with its top-left corner at screen (199,130); P x y gives the
    # screen position of picture pixel (x,y).
    P() { echo "$((199 + $1)) $((130 + $2))"; }
    drag() { xdotool mousemove $(P $1 $2) mousedown 1 mousemove $(P $(( ($1+$3)/2 )) $(( ($2+$4)/2 ))) mousemove $(P $3 $4) mouseup 1; }
    "$CARGO_TARGET_DIR/debug/obtura" --screen &
    APP=$!
    # A failed check must not leave the app running: the script would wait for it forever.
    trap "kill $APP 2>/dev/null" EXIT
    sleep 4
    titles() { xdotool search --name "^Obtura: " 2>/dev/null | wc -l; }
    OPEN_AT_START=$(titles)
    # The capture is on the clipboard before anything was touched.
    CLIP=$(xclip -selection clipboard -t image/png -o 2>/dev/null | identify -format "%wx%h" - 2>/dev/null || echo none)
    [ "$CLIP" = "${W}x${H}" ] || { echo "FAIL: the capture is not on the clipboard at once (got $CLIP)" >&2; exit 1; }
    # Arrow: the Line tool ends in an arrow head by default.
    xdotool key l; drag 60 60 260 60
    # Curve (S): drawn straight across, it bulges up by a fifth of its length; its middle
    # handle then drags it down, below the chord.
    xdotool key s; drag 60 200 260 200
    xdotool mousemove $(P 160 160) mousedown 1 mousemove $(P 160 200) mousemove $(P 160 240) mouseup 1
    # Callout (K): press on what it points at, drag to where the box goes, type, Enter.
    xdotool key k; drag 517 417 397 387
    sleep 0.5; xdotool type "Hi"; xdotool key Return
    # Step (N).
    xdotool key n; xdotool mousemove $(P 617 67) click 1
    # Round blur (B) over the big text, shape chosen in the Formatting panel.
    xdotool key b; xdotool key f; sleep 0.6
    xdotool mousemove 162 111 click 1; sleep 0.4; xdotool key Escape; sleep 0.3
    drag 157 222 397 287
    sleep 1; shot 1-marks
    xdotool key ctrl+s; sleep 1.5
    SAVED=$(ls "$HOME"/Pictures/Screenshots/Screenshot*.png 2>/dev/null | head -1 || true)
    [ -n "$SAVED" ] || { echo "FAIL: nothing was saved" >&2; exit 1; }
    rgb() { convert "$SAVED" -alpha off -crop 1x1+$1+$2 +repage -format "%[fx:int(255*r+0.5)] %[fx:int(255*g+0.5)] %[fx:int(255*b+0.5)]" info:; }
    is_red() { read -r r g b <<<"$(rgb $1 $2)"; [ "$r" -gt 190 ] && [ "$g" -lt 90 ] && [ "$b" -lt 90 ]; }
    is_white() { read -r r g b <<<"$(rgb $1 $2)"; [ "$r" -gt 240 ] && [ "$g" -gt 240 ] && [ "$b" -gt 240 ]; }
    is_red 150 60 || { echo "FAIL: no arrow line at 150,60 ($(rgb 150 60))" >&2; exit 1; }
    is_red 242 54 && is_red 242 66 || { echo "FAIL: the arrow has no head" >&2; exit 1; }
    is_red 100 200 && { echo "FAIL: the curve was not bent down by its handle" >&2; exit 1; }
    is_red 160 240 || { echo "FAIL: no curve through the point it was dragged to ($(rgb 160 240))" >&2; exit 1; }
    is_white 394 392 || { echo "FAIL: callout box is not white ($(rgb 394 392))" >&2; exit 1; }
    is_red 617 55 || { echo "FAIL: no step badge ($(rgb 617 55))" >&2; exit 1; }
    # The round blur: the corner of its rectangle is the original picture, its middle is not.
    differing() { convert "$SAVED" -crop $1 +repage "$SMOKE_OUT/a.png"; convert "$PNG" -crop $1 +repage "$SMOKE_OUT/b.png"; compare -metric AE "$SMOKE_OUT/a.png" "$SMOKE_OUT/b.png" null: 2>&1 || true; }
    CORNER=$(differing 6x6+158+223)
    MIDDLE=$(differing 80x30+237+240)
    CORNER=${CORNER%% *}; MIDDLE=${MIDDLE%% *}
    echo "differing pixels: corner $CORNER, middle $MIDDLE"
    [ "$CORNER" = 0 ] || { echo "FAIL: the blur reached the corner of its rectangle" >&2; exit 1; }
    [ "$MIDDLE" -gt 100 ] || { echo "FAIL: the blur did not change the picture" >&2; exit 1; }
    # New remembers the kind of the capture that opened this editor (a full screen), shown in
    # the chooser beside it; then Region is chosen there.
    KIND() { /usr/bin/gdbus call --session --dest com.grasshillsco.Obtura --object-path /com/grasshillsco/Obtura/window/$1 --method org.gtk.Actions.Describe new-kind 2>/dev/null | grep -o "<[^>]*>" | tr -d "<>"; }
    case "$(KIND 1)" in *screen*) ;; *) echo "FAIL: New does not remember the full screen capture (it takes $(KIND 1))" >&2; exit 1;; esac
    /usr/bin/gdbus call --session --dest com.grasshillsco.Obtura --object-path /com/grasshillsco/Obtura/window/1 --method org.gtk.Actions.Activate new-kind "[<\"region\">]" "{}" >/dev/null
    sleep 0.5; shot 1b-chooser
    case "$(KIND 1)" in *region*) ;; *) echo "FAIL: the chooser did not take Region (it says $(KIND 1))" >&2; exit 1;; esac
    # New screenshot with nothing unsaved: no question; cancelling the overlay keeps the window.
    xdotool key ctrl+n; sleep 2; shot 2-overlay
    xdotool key Escape; sleep 1.5
    [ "$(titles)" = "$OPEN_AT_START" ] || { echo "FAIL: cancelling a new screenshot changed the windows ($(titles) vs $OPEN_AT_START)" >&2; exit 1; }
    # Draw something: now "New" must ask first, and Cancel must change nothing.
    xdotool key r; drag 500 100 560 140; sleep 0.5
    xdotool key ctrl+n; sleep 1; shot 3-asks
    xdotool mousemove 557 463 click 1; sleep 1; shot 4-cancelled
    [ "$(xdotool search --name "^Obtura: select a region" 2>/dev/null | wc -l)" = 0 ] || { echo "FAIL: Cancel in the question went on to a new screenshot" >&2; exit 1; }
    # Discard Changes goes on to the overlay; accepting a region replaces this window.
    xdotool key ctrl+n; sleep 1; xdotool mousemove 557 407 click 1; sleep 2
    xdotool mousemove 100 100 mousedown 1 mousemove 300 250 mousemove 600 450 mouseup 1; sleep 1
    xdotool key Return; sleep 3; shot 5-replaced
    NOW=$(xdotool search --name "^Obtura: " 2>/dev/null | wc -l)
    OLD=$(xdotool search --name "^Obtura: 800 × 500" 2>/dev/null | wc -l)
    echo "windows: $NOW (was $OPEN_AT_START), still showing the old 800 x 500 picture: $OLD"
    [ "$NOW" = "$OPEN_AT_START" ] && [ "$OLD" = 0 ] || { echo "FAIL: the new screenshot did not replace the window" >&2; exit 1; }
    # The new window remembers too: a region.
    case "$(KIND 2)" in *region*) ;; *) echo "FAIL: the new editor does not take a region with New (it says $(KIND 2))" >&2; exit 1;; esac
    kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null || true
    echo "OK: capture on the clipboard at once; arrow, bent curve, callout, step and round blur in the saved file; New remembers its kind, asks and replaces"
  elif [ "$SMOKE_MODE" = phase2 ]; then
    P() { echo "$((199 + $1)) $((130 + $2))"; }
    drag() { xdotool mousemove $(P $1 $2) mousedown 1 mousemove $(P $(( ($1+$3)/2 )) $(( ($2+$4)/2 ))) mousemove $(P $3 $4) mouseup 1; }
    "$CARGO_TARGET_DIR/debug/obtura" --screen &
    APP=$!
    trap "kill $APP 2>/dev/null" EXIT
    sleep 4
    # Stamp (M): a red tick at (600,100).
    xdotool key m; xdotool mousemove $(P 600 100) click 1
    # Magnifier (Z) on the pink circle: the lens goes to its left, at (626..666, 400..440).
    xdotool key z; drag 690 410 710 430
    # Spotlight (D): everything but (300..500, 300..380) is dimmed.
    xdotool key d; drag 300 300 500 380
    # Paste a 60 x 60 green picture: placed in the middle, at (370..430, 220..280).
    convert -size 60x60 xc:"#00ff00" "$SMOKE_OUT/green.png"
    xclip -selection clipboard -t image/png -i "$SMOKE_OUT/green.png"; sleep 0.5
    xdotool key Escape key v key ctrl+v; sleep 1
    # Cut out (U) the rows 460..490: the picture becomes 470 high.
    xdotool key u; drag 100 460 102 490; sleep 0.5
    shot 1-marks
    # Beautify (Shift+B): its switch turns the frame on (the default frame).
    xdotool key shift+b; sleep 0.8; xdotool mousemove 920 92 click 1; sleep 0.5; shot 2-beautify; xdotool key Escape; sleep 0.5
    xdotool key ctrl+s; sleep 2
    SAVED=$(ls "$HOME"/Pictures/Screenshots/Screenshot*.png 2>/dev/null | head -1 || true)
    [ -n "$SAVED" ] || { echo "FAIL: nothing was saved" >&2; exit 1; }
    SIZE=$(identify -format "%wx%h" "$SAVED")
    echo "saved $SIZE"
    # 800 x 470 picture with 56 px of padding all round.
    [ "$SIZE" = "912x582" ] || { echo "FAIL: saved $SIZE, wanted 912x582 (800 x 470 picture in the frame)" >&2; exit 1; }
    rgb() { convert "$SAVED" -alpha off -crop 1x1+$1+$2 +repage -format "%[fx:int(255*r+0.5)] %[fx:int(255*g+0.5)] %[fx:int(255*b+0.5)]" info:; }
    orig() { convert "$PNG" -alpha off -crop 1x1+$1+$2 +repage -format "%[fx:int(255*r+0.5)] %[fx:int(255*g+0.5)] %[fx:int(255*b+0.5)]" info:; }
    at() { rgb $((56 + $1)) $((56 + $2)); }
    read -r r g b <<<"$(rgb 2 2)"; echo "corner $r $g $b"
    [ "$b" -gt 150 ] && [ "$r" -gt 60 ] || { echo "FAIL: no gradient background in the corner ($r $g $b)" >&2; exit 1; }
    read -r r g b <<<"$(at 400 250)"; [ "$r" -lt 20 ] && [ "$g" -gt 235 ] && [ "$b" -lt 20 ] || { echo "FAIL: the pasted picture is not at 400,250 ($r $g $b)" >&2; exit 1; }
    read -r r g b <<<"$(at 646 420)"; echo "lens $r $g $b"; [ "$r" -gt $((g + 40)) ] || { echo "FAIL: the lens does not show the pink circle ($r $g $b)" >&2; exit 1; }
    RED=0; for dx in -8 -4 0 4 8; do for dy in -8 -4 0 4 8; do read -r r g b <<<"$(at $((600+dx)) $((100+dy)))"; [ "$r" -gt $((g + 40)) ] && RED=$((RED+1)); done; done
    echo "stamp: $RED reddish points"; [ "$RED" -ge 3 ] || { echo "FAIL: no stamp at 600,100" >&2; exit 1; }
    read -r r1 g1 b1 <<<"$(at 400 340)"; read -r r0 g0 b0 <<<"$(orig 400 340)"
    [ "$r1" = "$r0" ] && [ "$b1" = "$b0" ] || { echo "FAIL: the lit area changed ($r1 $g1 $b1 vs $r0 $g0 $b0)" >&2; exit 1; }
    read -r r1 g1 b1 <<<"$(at 50 450)"; read -r r0 g0 b0 <<<"$(orig 50 480)"
    echo "dimmed $r1 $g1 $b1 (was about $r0 $g0 $b0)"; [ "$b1" -lt $((b0 * 6 / 10)) ] || { echo "FAIL: outside the spotlight is not dimmed" >&2; exit 1; }
    kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null || true
    echo "OK: stamp, magnifier, spotlight, pasted picture, strip cut out and frame are all in the saved file"
  elif [ "$SMOKE_MODE" = phase3 ]; then
    P() { echo "$((199 + $1)) $((130 + $2))"; }
    drag() { xdotool mousemove $(P $1 $2) mousedown 1 mousemove $(P $(( ($1+$3)/2 )) $(( ($2+$4)/2 ))) mousemove $(P $3 $4) mouseup 1; }
    "$CARGO_TARGET_DIR/debug/obtura" --screen &
    APP=$!
    trap "kill $APP 2>/dev/null" EXIT
    sleep 4
    newest() { ls -t "$HOME"/Pictures/Screenshots/Screenshot*.png 2>/dev/null | head -1; }
    rgb() { convert "$1" -alpha off -crop 1x1+$2+$3 +repage -format "%[fx:int(255*r+0.5)] %[fx:int(255*g+0.5)] %[fx:int(255*b+0.5)]" info:; }
    # An arrow from inside the picture to 30 px past its right edge (the window has room there).
    xdotool key l; drag 600 450 830 450; sleep 1; shot 1-past-the-edge
    xdotool key ctrl+s; sleep 1.5
    GROWN=$(newest); SIZE=$(identify -format "%wx%h" "$GROWN"); echo "with the arrow: $SIZE"
    [ "$SIZE" = "832x500" ] || { echo "FAIL: saved $SIZE, wanted 832x500 (30 px of arrow and its outline past the edge)" >&2; exit 1; }
    [ "$(rgb "$GROWN" 815 10)" = "255 255 255" ] || { echo "FAIL: the room added is not white ($(rgb "$GROWN" 815 10))" >&2; exit 1; }
    read -r r g b <<<"$(rgb "$GROWN" 815 450)"
    [ "$r" -gt 180 ] && [ "$g" -lt 80 ] || { echo "FAIL: the arrow head past the edge is missing ($r $g $b)" >&2; exit 1; }
    # The arrow is still selected: Delete removes it, and the room goes with it.
    sleep 1.1; xdotool key Delete; sleep 1; shot 2-deleted
    xdotool key ctrl+s; sleep 1.5
    SIZE=$(identify -format "%wx%h" "$(newest)"); echo "arrow deleted: $SIZE"
    [ "$SIZE" = "800x500" ] || { echo "FAIL: after deleting the arrow the picture is $SIZE, wanted 800x500" >&2; exit 1; }
    [ "$(compare -metric AE "$(newest)" "$PNG" null: 2>&1 | cut -d" " -f1)" = 0 ] || { echo "FAIL: after deleting the arrow the picture is not the screenshot" >&2; exit 1; }
    sleep 1.1; xdotool key ctrl+z; sleep 1; xdotool key ctrl+s; sleep 1.5
    SIZE=$(identify -format "%wx%h" "$(newest)"); echo "undone: $SIZE"
    [ "$SIZE" = "832x500" ] || { echo "FAIL: undoing the delete gave $SIZE, wanted 832x500" >&2; exit 1; }
    # Text typed so far that it runs past the right edge: the picture grows to hold it all.
    # (The view moved 16 px left to show the room for the arrow; a click near 700,30 is still on it.)
    xdotool key t; xdotool mousemove $(P 700 30) click 1; sleep 0.5
    xdotool type "A note that runs past the edge"; sleep 0.5; shot 1b-typing; xdotool key Return; sleep 1
    sleep 1.1; xdotool key ctrl+s; sleep 1.5
    SIZE=$(identify -format "%wx%h" "$(newest)"); echo "text past the edge: $SIZE"
    [ "${SIZE%x*}" -gt 1000 ] || { echo "FAIL: text typed past the edge gave $SIZE: the picture did not grow to hold it" >&2; exit 1; }
    echo "ok: an arrow past the edge adds white room, deleting it takes the room away, undo brings it back; text past the edge too"
    kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null || true
    rm -f "$HOME"/Pictures/Screenshots/*.png

    # A picture with things that must not be shared, a blue patch and a harmless line.
    SECRETS="$SMOKE_OUT/secrets.png"
    convert -size 800x500 xc:white -font DejaVu-Sans -pointsize 26 -fill black \
      -annotate +40+70 "Contact: jane.doe@example.com" -annotate +40+130 "Phone: +33 6 12 34 56 78" \
      -annotate +40+190 "Card: 4111 1111 1111 1111" -annotate +40+250 "Server 192.168.1.20" \
      -annotate +40+330 "Nothing secret here" -fill "#1c71d8" -draw "rectangle 600,370 760,440" "$SECRETS"
    OBTURA_FAKE_CAPTURE="$SECRETS" "$CARGO_TARGET_DIR/debug/obtura" --screen &
    APP=$!
    trap "kill $APP 2>/dev/null" EXIT
    sleep 4
    clip() { xclip -selection clipboard -o 2>/dev/null || true; }
    # Measure (Q): a 300 px dimension line along the bottom.
    xdotool key q; drag 100 470 400 470; sleep 0.5
    # Smart Redact (Ctrl+Shift+X): reads the text, lists what it found; Enter hides it all.
    xdotool key Escape key ctrl+shift+x; sleep 5; shot 3-smart-redact
    xdotool key Return; sleep 1; shot 4-hidden
    # Copy Text (Ctrl+Shift+C) reads the picture as redacted: nothing hidden comes back.
    xdotool key ctrl+shift+c; sleep 5
    TEXT=$(clip); echo "copied text: $(echo "$TEXT" | tr "\n" "|")"
    echo "$TEXT" | grep -q "Nothing secret here" || { echo "FAIL: Copy Text did not copy the harmless line" >&2; exit 1; }
    echo "$TEXT" | grep -q "Contact" || { echo "FAIL: Copy Text lost the words beside a hidden area" >&2; exit 1; }
    if echo "$TEXT" | grep -qE "jane|4111|192\.168|56 78"; then echo "FAIL: Copy Text reveals what was hidden" >&2; exit 1; fi
    # The Copy Text tool (G) on one line only.
    xdotool key g; drag 30 300 400 345; sleep 5
    TEXT=$(clip); echo "copied from the area: $TEXT"
    [ "$TEXT" = "Nothing secret here" ] || { echo "FAIL: the Copy Text tool copied \"$TEXT\"" >&2; exit 1; }
    # The colour picker (I) copies the colour clicked.
    xdotool key i; xdotool mousemove $(P 680 405); sleep 0.5; shot 5-loupe; xdotool click 1; sleep 1
    [ "$(clip)" = "#1c71d8" ] || { echo "FAIL: the colour picker copied \"$(clip)\", wanted #1c71d8" >&2; exit 1; }
    sleep 1.1; xdotool key ctrl+s; sleep 1.5
    SAVED=$(newest)
    [ -n "$SAVED" ] || { echo "FAIL: nothing was saved" >&2; exit 1; }
    mean() { convert "$1" -alpha off -crop $2 +repage -colorspace Gray -format "%[fx:mean]" info:; }
    for area in 250x16+180+55 200x14+150+113 250x14+140+173 150x14+150+233; do
      M=$(mean "$SAVED" $area); echo "mean brightness of the hidden $area: $M"
      awk "BEGIN{exit !($M < 0.02)}" || { echo "FAIL: $area is not hidden in the saved picture" >&2; exit 1; }
    done
    [ "$(convert "$SAVED" -crop 300x40+30+300 +repage miff:- | compare -metric AE - <(convert "$SECRETS" -crop 300x40+30+300 +repage miff:-) null: 2>&1 | cut -d" " -f1)" = 0 ] || { echo "FAIL: the harmless line changed" >&2; exit 1; }
    read -r r g b <<<"$(rgb "$SAVED" 216 470)"
    [ "$b" -gt 150 ] && [ "$r" -lt 80 ] || { echo "FAIL: no measure label at the middle of the line ($r $g $b)" >&2; exit 1; }
    # The library: the capture is there, its text was read, the saved edits (marks) are kept.
    sleep 2
    ITEM=$(ls -d "$XDG_DATA_HOME"/obtura/library/*/ | tail -1)
    grep -q "Nothing secret" "$ITEM/item.json" || { echo "FAIL: the library did not read the text of the capture" >&2; cat "$ITEM/item.json" >&2; exit 1; }
    sleep 3
    if grep -qE "jane|4111" "$ITEM/item.json"; then echo "FAIL: the library search text still holds what was hidden" >&2; exit 1; fi
    grep -q "\"edited\": true" "$ITEM/item.json" && grep -q "Redact" "$ITEM/marks.json" || { echo "FAIL: the library did not keep the edits" >&2; exit 1; }
    [ "$(compare -metric AE "$ITEM/original.png" "$SECRETS" null: 2>&1 | cut -d" " -f1)" = 0 ] || { echo "FAIL: the library original is not the capture" >&2; exit 1; }
    # The library window: Ctrl+L, a search, a click opens the capture again with its marks.
    xdotool key ctrl+l; sleep 2; shot 6-library
    xdotool type "secret"; sleep 1.5; shot 7-library-search
    LIB=$(xdotool search --name "^Obtura Library" | head -1)
    [ -n "$LIB" ] || { echo "FAIL: Ctrl+L did not open the library" >&2; exit 1; }
    eval "$(xdotool getwindowgeometry --shell "$LIB")"
    xdotool mousemove $((X + 130)) $((Y + 140)) click 1; sleep 2; shot 8-reopened
    EDITORS=$(xdotool search --name "^Obtura: 800 × 500" 2>/dev/null | wc -l)
    [ "$EDITORS" = 1 ] || { echo "FAIL: clicking the library item gave $EDITORS editors (its own editor is already open: it should come forward, not open twice)" >&2; exit 1; }
    # Close the editor (it was saved: no question), open the capture again from the library:
    # its marks come back.
    xdotool windowfocus "$(xdotool search --name "^Obtura: 800 × 500" | head -1)"; sleep 0.3; xdotool key ctrl+w; sleep 1
    [ "$(xdotool search --name "^Obtura: 800 × 500" 2>/dev/null | wc -l)" = 0 ] || { echo "FAIL: the saved editor did not close" >&2; exit 1; }
    xdotool mousemove $((X + 130)) $((Y + 140)) click 1; sleep 2.5; shot 9-reopened-from-library
    AGAIN=$(xdotool search --name "^Obtura: 800 × 500" 2>/dev/null | head -1)
    [ -n "$AGAIN" ] || { echo "FAIL: the library did not open the capture again" >&2; exit 1; }
    sleep 1.1; xdotool windowfocus "$AGAIN"; sleep 0.3; xdotool key ctrl+s; sleep 1.5
    M=$(mean "$(newest)" 250x16+180+55); echo "reopened and saved: hidden email brightness $M"
    awk "BEGIN{exit !($M < 0.02)}" || { echo "FAIL: the reopened capture lost its marks" >&2; exit 1; }
    kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null || true
    echo "OK: room past the edge; smart redact hid the email, phone, card and IP; Copy Text (all and area) never shows them; colour picker; measure; library (text read, edits kept, window, search)"
  elif [ "$SMOKE_MODE" = record ]; then
    BIN="$CARGO_TARGET_DIR/debug/obtura"
    FAILED=0
    check() { if [ "$2" = "$3" ]; then echo "ok: $1"; else echo "FAIL: $1: wanted $2, got $3" >&2; FAILED=1; fi; }
    settings() { mkdir -p "$XDG_CONFIG_HOME/obtura"; echo "{\"record\": $1}" > "$XDG_CONFIG_HOME/obtura/settings.json"; }
    info() { /usr/bin/gst-discoverer-1.0 -v "$1" 2>/dev/null; }
    secs() { info "$1" | sed -n "s/^ *Duration: \([0-9]*\):\([0-9]*\):\([0-9.]*\).*/\1 \2 \3/p" | awk "{printf \"%.1f\", \$1*3600+\$2*60+\$3}"; }
    within() { awk "BEGIN{exit !($1 >= $2 && $1 <= $3)}" && echo yes || echo "no ($1)"; }
    # GStreamer reads its plugins once into this throwaway home (slow the first time).
    /usr/bin/gst-inspect-1.0 --exists pipewiresrc
    # 1. A whole screen as MP4; running --record again stops it; the first command prints the
    # file. It is the running instance: it stays (its window is open), so it is not waited for.
    settings "{\"format\": \"mp4\", \"countdown\": 0}"
    "$BIN" --record --screen >"$SMOKE_OUT/rec1.txt" 2>"$SMOKE_OUT/rec1.err" &
    APP=$!
    trap "kill $APP 2>/dev/null" EXIT
    sleep 5; "$BIN" --record; check "second --record exit status" 0 $?
    for i in $(seq 1 40); do [ -s "$SMOKE_OUT/rec1.txt" ] && break; sleep 0.5; done
    F=$(cat "$SMOKE_OUT/rec1.txt"); echo "saved: $F $(cat "$SMOKE_OUT/rec1.err")"
    check "MP4 saved in Videos/Screencasts" yes "$(case "$F" in "$HOME"/Videos/Screencasts/Recording*.mp4) [ -s "$F" ] && echo yes;; *) echo "no: $F";; esac)"
    check "MP4 length" yes "$(within "$(secs "$F")" 2 4)"
    check "MP4 size" yes "$(info "$F" | grep -q "Width: ${W}" && info "$F" | grep -q "Height: ${H}" && echo yes || echo no)"
    sleep 1; shot rec-saved
    check "saved window" 1 "$(xdotool search --name "^Obtura: recording saved" 2>/dev/null | wc -l)"
    # Trim, in the window that shows the saved file: one second off the start and one off the
    # end (sliders moved with the keys); the file is 2 s shorter, the untrimmed one in the bin.
    BEFORE=$(secs "$F")
    xdotool mousemove 203 348 click 1; sleep 3; shot rec-trim
    xdotool mousemove 114 274 click 1 key --repeat 10 --delay 40 Right
    xdotool mousemove 532 314 click 1 key --repeat 10 --delay 40 Left
    sleep 0.5; shot rec-trim-set
    xdotool mousemove 400 400 click 1; sleep 5; shot rec-trimmed
    AFTER=$(secs "$F")
    echo "length before trimming $BEFORE s, after $AFTER s"
    check "trimmed by 2 seconds" yes "$(awk "BEGIN{d=$BEFORE-$AFTER; if (d>1.6 && d<2.4) print \"yes\"; else print \"no\"}")"
    check "the untrimmed recording is in the wastebasket" 1 "$(ls "$XDG_DATA_HOME"/Trash/files/Recording*.mp4 2>/dev/null | wc -l)"
    # 2. An area as WebM with both sounds mixed (test tones from the fake backend).
    settings "{\"format\": \"webm\", \"countdown\": 0, \"microphone\": true, \"computer_sound\": true}"
    "$BIN" --record >"$SMOKE_OUT/rec2.txt" 2>/dev/null &
    REC=$!
    sleep 2.5; shot rec-overlay
    xdotool mousemove 100 100 mousedown 1 mousemove 250 200 mousemove 401 301 mouseup 1; sleep 0.5; xdotool key Return
    sleep 3; "$BIN" --record; wait $REC; check "area recording exit status" 0 $?
    F=$(cat "$SMOKE_OUT/rec2.txt"); echo "saved: $F"
    check "WebM size is the area (even)" yes "$(info "$F" | grep -q "Width: 300" && info "$F" | grep -q "Height: 200" && echo yes || echo no)"
    check "WebM has sound" yes "$(info "$F" | grep -q "audio codec" && echo yes || echo no)"
    check "WebM length" yes "$(within "$(secs "$F")" 1.5 3.5)"
    # 3. Pause by opening Obtura, Resume in the window that appears: two parts joined.
    settings "{\"format\": \"mp4\", \"countdown\": 0, \"microphone\": true}"
    "$BIN" --record --screen >"$SMOKE_OUT/rec3.txt" 2>/dev/null &
    REC=$!
    sleep 2.5; "$BIN"; sleep 1.5; shot rec-paused
    PAUSED=$(xdotool search --name "^Obtura: recording paused" 2>/dev/null | head -1)
    check "pausing shows the controls" yes "$([ -n "$PAUSED" ] && echo yes || echo no)"
    sleep 2
    [ -n "$PAUSED" ] && xdotool windowfocus --sync "$PAUSED" key Return
    sleep 2.5; "$BIN" --record; wait $REC; check "paused recording exit status" 0 $?
    F=$(cat "$SMOKE_OUT/rec3.txt"); echo "saved: $F"
    check "the pause is not in the recording" yes "$(within "$(secs "$F")" 3 5.2)"
    check "joined MP4 has sound" yes "$(info "$F" | grep -q "audio codec" && echo yes || echo no)"
    # 4. The top-bar icon (a stand-in for the AppIndicator host): the time beside it, and its
    # menu pauses, resumes and stops; the pause is left out of the file.
    /usr/bin/python3 "$PWD/scripts/fake_tray_host.py" "$SMOKE_OUT/tray.log" &
    HOST=$!
    sleep 1
    settings "{\"format\": \"mp4\", \"countdown\": 0}"
    "$BIN" --record --screen >"$SMOKE_OUT/rec5.txt" 2>/dev/null &
    REC=$!
    item() { /usr/bin/gdbus call --session --dest com.grasshillsco.Obtura --object-path /StatusNotifierItem --method org.freedesktop.DBus.Properties.Get org.kde.StatusNotifierItem "$1" 2>/dev/null; }
    menu() { /usr/bin/gdbus call --session --dest com.grasshillsco.Obtura --object-path /StatusNotifierItem/Menu --method com.canonical.dbusmenu.Event "$1" clicked "<0>" 0 >/dev/null 2>&1; }
    sleep 2.5
    check "the icon registered with the top bar" yes "$(grep -q registered "$SMOKE_OUT/tray.log" && echo yes || echo no)"
    check "the icon is shown" "(<Active>,)" "$(item Status | tr -d "\"\047")"
    LABEL=$(item XAyatanaLabel); echo "label while recording: $LABEL"
    check "the time is beside the icon" yes "$(echo "$LABEL" | grep -q "0:0[1-9]" && echo yes || echo no)"
    menu 1; sleep 1.5
    LABEL=$(item XAyatanaLabel); echo "label after Pause: $LABEL"
    check "Pause in the menu pauses" yes "$(echo "$LABEL" | grep -q "Paused" && echo yes || echo no)"
    check "the paused icon" yes "$(item IconName | grep -q pause && echo yes || echo no)"
    sleep 2
    menu 1; sleep 2.5
    menu 2; wait $REC; check "Stop and Save in the menu saves" 0 $?
    F=$(cat "$SMOKE_OUT/rec5.txt"); echo "saved: $F"
    check "the menu pause is not in the recording" yes "$(within "$(secs "$F")" 3 5.2)"
    check "the icon is hidden after the recording" "(<Passive>,)" "$(item Status | tr -d "\"\047")"
    kill $HOST 2>/dev/null
    # 5. A GIF at half size.
    settings "{\"format\": \"gif\", \"countdown\": 0, \"gif_scale\": 50}"
    "$BIN" --record --screen >"$SMOKE_OUT/rec4.txt" 2>/dev/null &
    REC=$!
    sleep 3; "$BIN" --record; wait $REC; check "GIF recording exit status" 0 $?
    F=$(cat "$SMOKE_OUT/rec4.txt"); echo "saved: $F"
    check "GIF size" "$((W/2))x$((H/2))" "$(identify -format "%Wx%H\n" "$F" 2>/dev/null | head -1)"
    check "GIF has several frames" yes "$([ "$(identify "$F" 2>/dev/null | wc -l)" -ge 5 ] && echo yes || echo no)"
    # Trim the GIF: one second off the end.
    gif_cs() { identify -format "%T\n" "$1" 2>/dev/null | awk "{s+=\$1} END {print s}"; }
    BEFORE=$(gif_cs "$F")
    sleep 1; xdotool mousemove 203 348 click 1; sleep 3
    xdotool mousemove 532 314 click 1 key --repeat 10 --delay 40 Left
    sleep 0.5; xdotool mousemove 400 400 click 1; sleep 4; shot rec-gif-trimmed
    AFTER=$(gif_cs "$F")
    echo "GIF length before trimming $BEFORE, after $AFTER hundredths"
    check "GIF trimmed by 1 second" yes "$(awk "BEGIN{d=$BEFORE-$AFTER; if (d>80 && d<120) print \"yes\"; else print \"no\"}")"
    check "scratch files removed" 0 "$(ls -d "$XDG_CACHE_HOME"/obtura/recording-* 2>/dev/null | wc -l)"
    kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null || true
    [ "$FAILED" = 0 ] && echo "OK: recording (MP4 screen, WebM area with sound, pause and resume, GIF)" || { echo "FAIL: see above" >&2; exit 1; }
  elif [ "$SMOKE_MODE" = edit ]; then
    "$CARGO_TARGET_DIR/debug/obtura" --screen &
    APP=$!
    sleep 4; shot 1-editor
    # Rectangle (R): drag in the picture. Pointer positions are screen pixels.
    xdotool key r
    xdotool mousemove 372 247 mousedown 1 mousemove 472 297 mousemove 572 347 mouseup 1
    # Numbered step (N): one click.
    xdotool key n
    xdotool mousemove 772 247 click 1
    # Box (O) over the big text in the middle of the picture.
    xdotool key o
    xdotool mousemove 302 352 mousedown 1 mousemove 572 377 mousemove 772 407 mouseup 1
    sleep 1; shot 2-marks
    xdotool key ctrl+s
    sleep 1; shot 3-saved
    SAVED=$(ls "$HOME"/Pictures/Screenshots/Screenshot*.png 2>/dev/null | head -1 || true)
    kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null || true
    [ -n "$SAVED" ] || { echo "FAIL: nothing was saved" >&2; exit 1; }
    SIZE=$(identify -format "%wx%h" "$SAVED")
    [ "$SIZE" = "${W}x${H}" ] || { echo "FAIL: saved picture is $SIZE, wanted ${W}x${H}" >&2; exit 1; }
    # Bounding box of the red of the rectangle (the picture has no such red of its own).
    RED=$(convert "$SAVED" -fuzz 6% -fill white +opaque "#e01b24" -fill black -opaque "#e01b24" -trim -format "%wx%h" info: 2>/dev/null || echo none)
    echo "red marks bounding box: $RED"
    case "$RED" in none|"") echo "FAIL: no red marks in the saved picture" >&2; exit 1;; esac
    # The covered strip of the picture must be pure black now: no trace of the white text.
    MEAN=$(convert "$SAVED" -alpha off -crop 150x20+250+235 +repage -colorspace Gray -format "%[fx:mean]" info:)
    echo "mean brightness inside the box: $MEAN"
    awk "BEGIN{exit !($MEAN < 0.01)}" || { echo "FAIL: the box did not cover the text" >&2; exit 1; }
    echo "OK: marks drawn, box destroyed the text, picture saved as $SIZE"
  elif [ "$SMOKE_MODE" = scroll ]; then
    BIN="$CARGO_TARGET_DIR/debug/obtura"
    FAILED=0
    check() { # check "what" expected actual
      if [ "$2" = "$3" ]; then echo "ok: $1"; else echo "FAIL: $1: wanted $2, got $3" >&2; FAILED=1; fi
    }
    trap "pkill -x obtura 2>/dev/null" EXIT
    PAGE="$PWD/tests/fixtures/long-page.png"
    # The fake screen shows the page (header fixed) and scrolls down 40 rows 8 times a second
    # while the capture runs: about 7 s to its bottom.
    OBTURA_FAKE_SCROLL="$PAGE" "$BIN" --scroll --save "$SMOKE_OUT/long.png" >/dev/null 2>&1 &
    CLI=$!
    sleep 3; shot scroll-1-overlay
    check "the overlay asks what scrolls" 1 "$(xdotool search --onlyvisible --name "^Obtura: select what scrolls" 2>/dev/null | wc -l)"
    xdotool mousemove 100 70 mousedown 1 mousemove 400 300 mousemove 700 480 mouseup 1; sleep 0.5
    xdotool key Return
    sleep 4; shot scroll-2-running
    sleep 10
    # Running --scroll again finishes the capture under way.
    OBTURA_FAKE_SCROLL="$PAGE" "$BIN" --scroll >/dev/null 2>&1; check "--scroll again exit status" 0 $?
    wait $CLI; check "--scroll --save exit status" 0 $?
    check "the joined picture is the page from the top to the bottom of the part chosen" "600x2510" "$(identify -format "%wx%h" "$SMOKE_OUT/long.png" 2>/dev/null || echo none)"
    convert "$PAGE" -crop 600x2510+100+70 +repage "$SMOKE_OUT/long-expected.png"
    check "the joined picture is exactly the page" 0 "$(compare -metric AE "$SMOKE_OUT/long.png" "$SMOKE_OUT/long-expected.png" null: 2>&1 | cut -d" " -f1)"
    check "scrolling frames were removed" 0 "$(ls -d "$XDG_CACHE_HOME"/obtura/scrolling-* 2>/dev/null | wc -l)"
    # From an open editor (its New menu: the window action new-scrolling): the editor leaves
    # the screen while the page scrolls (a see-through one would catch the wheel and the keys),
    # and the joined picture replaces it.
    OBTURA_FAKE_SCROLL="$PAGE" "$BIN" --screen >/dev/null 2>&1 &
    sleep 4
    check "an editor is open" 1 "$(xdotool search --onlyvisible --name "^Obtura: ${W} × ${H}" 2>/dev/null | wc -l)"
    /usr/bin/gdbus call --session --dest com.grasshillsco.Obtura --object-path /com/grasshillsco/Obtura/window/1 --method org.gtk.Actions.Activate new-scrolling "[]" "{}" >/dev/null
    sleep 3
    xdotool mousemove 100 70 mousedown 1 mousemove 400 300 mousemove 700 480 mouseup 1; sleep 0.5
    xdotool key Return
    sleep 3; shot scroll-3-from-editor
    check "the editor is off the screen while the page scrolls" 0 "$(xdotool search --onlyvisible --name "^Obtura: ${W} × ${H}" 2>/dev/null | wc -l)"
    sleep 4
    OBTURA_FAKE_SCROLL="$PAGE" "$BIN" --scroll >/dev/null 2>&1; check "--scroll again finishes the one started from the editor" 0 $?
    sleep 3; shot scroll-4-replaced
    check "the old editor is gone" 0 "$(xdotool search --name "^Obtura: ${W} × ${H}" 2>/dev/null | wc -l)"
    check "the joined picture opened in an editor" 1 "$(xdotool search --onlyvisible --name "^Obtura: 600 × " 2>/dev/null | wc -l)"
    pkill -x obtura 2>/dev/null || true
    sleep 1
    # --measure: a drag on the frozen screen; Enter copies and prints the measurement. Having
    # copied, the instance stays running for the clipboard (started with &).
    "$BIN" --measure >"$SMOKE_OUT/measure.txt" 2>/dev/null &
    sleep 3; xdotool mousemove 100 100 mousedown 1 mousemove 250 175 mousemove 400 250 mouseup 1; sleep 0.5; shot measure
    xdotool key Return; sleep 1.5
    check "--measure printed the measurement" "300 × 150 px, 335 px long at 333°" "$(cat "$SMOKE_OUT/measure.txt")"
    check "--measure copied it" "300 × 150 px, 335 px long at 333°" "$(xclip -selection clipboard -o 2>/dev/null)"
    pkill -x obtura 2>/dev/null || true
    sleep 1
    [ "$FAILED" = 0 ] && echo "OK: scrolling capture and measuring behave" || { echo "FAIL: see above" >&2; exit 1; }
  elif [ "$SMOKE_MODE" = extras ]; then
    BIN="$CARGO_TARGET_DIR/debug/obtura"
    FAILED=0
    check() { # check "what" expected actual
      if [ "$2" = "$3" ]; then echo "ok: $1"; else echo "FAIL: $1: wanted $2, got $3" >&2; FAILED=1; fi
    }
    P() { echo "$((199 + $1)) $((130 + $2))"; }
    drag() { xdotool mousemove $(P $1 $2) mousedown 1 mousemove $(P $(( ($1+$3)/2 )) $(( ($2+$4)/2 ))) mousemove $(P $3 $4) mouseup 1; }
    newest() { ls -t "$HOME"/Pictures/Screenshots/Screenshot*.png 2>/dev/null | head -1; }
    is_red() { read -r r g b <<<"$(convert "$1" -alpha off -crop 1x1+$2+$3 +repage -format "%[fx:int(255*r+0.5)] %[fx:int(255*g+0.5)] %[fx:int(255*b+0.5)]" info:)"; [ "$r" -gt 170 ] && [ "$g" -lt 90 ] && [ "$b" -lt 90 ]; }
    "$BIN" --screen &
    APP=$!
    trap "kill $APP 2>/dev/null; pkill -x obtura 2>/dev/null" EXIT
    sleep 4
    # A long thin rectangle, turned a quarter by its round handle (26 px above its top edge).
    xdotool key r; drag 100 350 300 370; sleep 0.5
    xdotool mousemove $(P 200 322) mousedown 1 mousemove $(P 225 328) mousemove $(P 238 345) mousemove $(P 238 360) mouseup 1
    sleep 0.6; shot extras-1-turned
    # A line, then with the Select tool a double-click on it adds a bend, dragged down.
    xdotool key l; drag 400 420 700 420; sleep 0.5
    xdotool key v; xdotool mousemove $(P 550 420) click 1; sleep 0.4
    xdotool mousemove $(P 550 420) click --repeat 2 --delay 90 1; sleep 0.5
    xdotool mousemove $(P 550 420) mousedown 1 mousemove $(P 550 440) mousemove $(P 550 470) mouseup 1
    sleep 0.6; shot extras-2-bent
    # Text on the clipboard, pasted where the pointer is.
    printf "Pasted note" | xclip -selection clipboard; sleep 0.3
    xdotool mousemove $(P 420 60); xdotool key ctrl+v; sleep 0.8; shot extras-3-pasted
    xdotool key Escape; sleep 0.3
    xdotool key ctrl+s; sleep 1.5
    SAVED=$(newest)
    [ -n "$SAVED" ] || { echo "FAIL: nothing was saved" >&2; exit 1; }
    check "saved at the screenshot size" "${W}x${H}" "$(identify -format "%wx%h" "$SAVED")"
    is_red "$SAVED" 190 300 && check "the turned rectangle stands up" yes yes || check "the turned rectangle stands up" yes no
    is_red "$SAVED" 290 352 && check "the rectangle left its old place" yes no || check "the rectangle left its old place" yes yes
    is_red "$SAVED" 550 470 && check "the line bends through the dragged bend" yes yes || check "the line bends through the dragged bend" yes no
    is_red "$SAVED" 550 420 && check "the line left its straight path" yes no || check "the line left its straight path" yes yes
    convert "$SAVED" -crop 140x40+420+60 +repage "$SMOKE_OUT/pasted.png"; convert "$PNG" -crop 140x40+420+60 +repage "$SMOKE_OUT/pasted-before.png"
    CHANGED=$(compare -metric AE "$SMOKE_OUT/pasted.png" "$SMOKE_OUT/pasted-before.png" null: 2>&1 | cut -d" " -f1)
    check "the pasted text is on the picture" yes "$([ "${CHANGED%.*}" -gt 150 ] 2>/dev/null && echo yes || echo "no ($CHANGED pixels changed)")"
    kill $APP 2>/dev/null; wait $APP 2>/dev/null || true
    # Read QR Codes in the editor: one code, copied at once.
    OBTURA_FAKE_CAPTURE="$PWD/tests/fixtures/qr-one.png" "$BIN" --screen &
    APP=$!
    sleep 4; xdotool key ctrl+shift+q; sleep 2.5; shot extras-4-qr
    check "Read QR Codes copied the link" "https://example.org/obtura" "$(xclip -selection clipboard -o 2>/dev/null)"
    kill $APP 2>/dev/null; wait $APP 2>/dev/null || true
    [ "$FAILED" = 0 ] && echo "OK: turning, bends, pasted text and QR reading behave" || { echo "FAIL: see above" >&2; exit 1; }
  else
    "$CARGO_TARGET_DIR/debug/obtura" --screen &
    APP=$!
    sleep 6; shot screen
    if kill -0 "$APP" 2>/dev/null; then
      kill "$APP"; wait "$APP" 2>/dev/null || true
      echo "OK: app ran for 6 s. Screenshot: $SMOKE_OUT/screen.png"
    else
      wait "$APP" || true; echo "FAIL: app exited early" >&2; exit 1
    fi
  fi
' _ 
