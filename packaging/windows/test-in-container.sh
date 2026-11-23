#!/usr/bin/env bash
# Runs inside the obtura-windows-build container (see test-exe.sh): unpacks the portable zip
# and runs the real obtura.exe under Wine, on a virtual X screen (Xvfb, 1280 x 800).
#  1. --version and --help answer (before GTK starts)
#  2. --screen --save takes a real screenshot through Windows' GDI (Wine's screen): a PNG of
#     the screen's size
#  3. --ocr reads the text of a picture with the bundled Tesseract (fake capture backend)
#  4. --qr reads a QR code (fake capture backend)
#  5. a region dragged on the overlay is saved at the size dragged
#  6. "A Window" shows the window picker; the Notepad clicked (behind another) is captured
#  7. a scrolling capture of Notepad, scrolled with the wheel, finished with Print Screen
#  8. Print Screen, a key in every app, opens the overlay
#  9. the start window and the editor open; screenshots of them go to /out/*.png to look at;
#     a colour picked in Format › More formatting with one double-click
# 10. the installer installs for the user, and its uninstaller removes only its own files
# Wine is not Windows: it checks that the program, its DLLs and its data files work
# together, not how it looks or behaves on a real Windows desktop.
set -uo pipefail
ZIP=$(ls /out/Obtura-*-windows-x64.zip | tail -1)
VERSION=$(basename "$ZIP" | sed -E 's/Obtura-(.*)-windows-x64.zip/\1/')
T=/tmp/t
rm -rf "$T" && mkdir -p "$T" && cd "$T" && unzip -q "$ZIP"
BIN="$T/Obtura/bin/obtura.exe"
export WINEPREFIX=$T/wine WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml=" DISPLAY=:99
# Wine's DirectWrite draws GTK's text as garbage (or not at all); real Windows uses its own,
# complete one. Under Wine, Pango is told to use fontconfig, so the pictures can be read.
export PANGOCAIRO_BACKEND=fc
# Obtura's settings and library go into the throwaway Wine profile, never anywhere else.
Xvfb :99 -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 &
XVFB=$!
trap 'wineserver -k 2>/dev/null; kill $XVFB 2>/dev/null' EXIT
sleep 1
wineboot -i >/dev/null 2>&1
FAILED=0
check() { # name expected actual
  if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1: expected '$2', got '$3'"; FAILED=1; fi
}
win() { winepath -w "$1" 2>/dev/null; }

# 1. Answers before GTK.
OUT=$(timeout 60 wine "$BIN" --version 2>/dev/null | tr -d '\r')
check "--version" "obtura $VERSION" "$OUT"
OUT=$(timeout 60 wine "$BIN" --help 2>/dev/null | tr -d '\r' | head -1)
check "--help" "Usage: obtura [OPTIONS]" "$OUT"
timeout 60 wine "$BIN" --bogus >/dev/null 2>&1
check "a wrong option exits with 2" 2 $?

# 2. A real screenshot through GDI.
mkdir -p "$T/shots"
# A coloured window on the screen, so the picture holds something known.
timeout 120 wine "$BIN" --screen --save "$(win "$T/shots/screen.png")" >"$T/save.log" 2>&1
check "--screen --save exits with 0" 0 $?
SIZE=$(identify -format '%wx%h' "$T/shots/screen.png" 2>/dev/null)
check "the screenshot is the screen's size" 1280x800 "$SIZE"
cp "$T/shots/screen.png" /out/wine-screen.png 2>/dev/null

# 3. Text reading with the bundled Tesseract (the picture comes from the fake backend).
OBTURA_FAKE_CAPTURE="$(win /src/tests/fixtures/sample.png)" timeout 120 wine "$BIN" --screen --ocr >"$T/ocr.txt" 2>/dev/null &
for _ in $(seq 60); do grep -q . "$T/ocr.txt" 2>/dev/null && break; sleep 1; done
sleep 1
check "--ocr read the picture's text" yes "$(grep -qi fixture "$T/ocr.txt" && echo yes || echo "no: $(tr -d '\r' <"$T/ocr.txt" | head -3)")"
wineserver -k 2>/dev/null; sleep 1

# 4. QR codes.
OBTURA_FAKE_CAPTURE="$(win /src/tests/fixtures/qr-one.png)" timeout 120 wine "$BIN" --screen --qr >"$T/qr.txt" 2>/dev/null &
for _ in $(seq 60); do grep -q . "$T/qr.txt" 2>/dev/null && break; sleep 1; done
sleep 1
check "--qr read the code" yes "$(grep -q . "$T/qr.txt" && echo yes || echo no)"
echo "     QR code holds: $(tr -d '\r' <"$T/qr.txt" | head -1)"
wineserver -k 2>/dev/null; sleep 1

# 5. A region chosen on the overlay (real screen, through GDI): drag, then Enter.
timeout 120 wine "$BIN" --region --save "$(win "$T/shots/region.png")" >/dev/null 2>&1 &
CLI=$!
sleep 12
xdotool mousemove 100 100 mousedown 1 mousemove 250 200 mousemove 400 300 mouseup 1
sleep 1
xdotool key Return
wait $CLI
check "--region --save exits with 0" 0 $?
check "the region is the size dragged" 300x200 "$(identify -format '%wx%h' "$T/shots/region.png" 2>/dev/null)"
wineserver -k 2>/dev/null; sleep 1

# 6. A window, picked on Obtura's overlay: two Notepads of different sizes; the one behind is
#    clicked, so the picture must be that one (not the one in front).
cp /src/LICENSE "$T/front.txt"
wine notepad >/dev/null 2>&1 &
sleep 6
BACK=$(xdotool search --name 'Notepad' 2>/dev/null | head -1)
xdotool windowsize "$BACK" 420 300 windowmove "$BACK" 60 60 2>/dev/null; sleep 1
wine notepad "$(win "$T/front.txt")" >/dev/null 2>&1 &
sleep 6
FRONT=$(xdotool search --name 'front' 2>/dev/null | head -1)
xdotool windowsize "$FRONT" 560 380 windowmove "$FRONT" 640 300 2>/dev/null; sleep 1
timeout 120 wine "$BIN" --window --save "$(win "$T/shots/window.png")" >/dev/null 2>&1 &
CLI=$!
sleep 12
check "A Window shows the window picker" yes "$(xdotool search --name 'choose the window' >/dev/null 2>&1 && echo yes || echo "no: $(xdotool search --name . getwindowname %@ 2>/dev/null | sort -u | tr '\n' '|')")"
eval "$(xdotool getwindowgeometry --shell "$BACK" 2>/dev/null)"
xdotool mousemove $((X + WIDTH / 2)) $((Y + HEIGHT / 2)); sleep 1
import -window root /out/wine-window-picker.png
xdotool click 1
wait $CLI
check "--window --save exits with 0" 0 $?
SHOT=$(identify -format '%wx%h' "$T/shots/window.png" 2>/dev/null)
echo "     the Notepad clicked is ${WIDTH:-?}x${HEIGHT:-?} on the screen, the capture $SHOT"
check "the window capture is the Notepad clicked (not the one in front)" yes "$(python3 -c "
w,h='$SHOT'.split('x'); print('yes' if abs(int(w)-${WIDTH:-0})<=40 and abs(int(h)-${HEIGHT:-0})<=60 else 'no')" 2>/dev/null || echo no)"
cp "$T/shots/window.png" /out/wine-window.png 2>/dev/null
wineserver -k 2>/dev/null; sleep 1

# 7. A scrolling capture of Notepad holding a long text: select its text, scroll it with the
#    wheel, finish with Print Screen. The joined picture must be much taller than the part.
# (Real text: lines that differ only by a number look alike to the joiner, as on Linux.)
cp /src/LICENSE "$T/long.txt"
wine notepad "$(win "$T/long.txt")" >/dev/null 2>&1 &
sleep 6
NOTEPAD=$(xdotool search --name 'long' 2>/dev/null | head -1)
xdotool windowmove "$NOTEPAD" 300 100 2>/dev/null; sleep 1
eval "$(xdotool getwindowgeometry --shell "$NOTEPAD" 2>/dev/null)"
timeout 200 wine "$BIN" --scroll --save "$(win "$T/shots/scroll.png")" >/dev/null 2>&1 &
CLI=$!
sleep 12
# The text part of Notepad (below its menu, left of its scroll bar).
SX=$((X + 20)); SY=$((Y + 80)); EX=$((X + WIDTH - 60)); EY=$((Y + HEIGHT - 80))
xdotool mousemove $SX $SY mousedown 1 mousemove $(( (SX+EX)/2 )) $(( (SY+EY)/2 )) mousemove $EX $EY mouseup 1
sleep 1
xdotool key Return
sleep 4
# (Wine sends the wheel to the window with the focus: a click in the text first.)
xdotool mousemove $(( (SX+EX)/2 )) $(( (SY+EY)/2 )) click 1
sleep 0.5
for _ in $(seq 25); do xdotool click 5; sleep 0.4; done
sleep 2
xdotool key Print
wait $CLI
check "--scroll --save exits with 0" 0 $?
PART=$((EY - SY))
TALL=$(identify -format '%h' "$T/shots/scroll.png" 2>/dev/null || echo 0)
echo "     part selected ${PART} px high, joined picture ${TALL:-0} px high"
check "the scrolling capture joined more than the part" yes "$([ "${TALL:-0}" -gt $((PART * 3 / 2)) ] && echo yes || echo no)"
cp "$T/shots/scroll.png" /out/wine-scroll.png 2>/dev/null
wineserver -k 2>/dev/null; sleep 1

# 8. The start window, then Print Screen (a key in every app) opens the overlay.
timeout 300 wine "$BIN" >/dev/null 2>&1 &
sleep 20
import -window root /out/wine-start-window.png
check "the start window opened" yes "$(xdotool search --name '^Obtura$' >/dev/null 2>&1 && echo yes || echo no)"
xdotool key Print
sleep 6
check "Print Screen opened the overlay" yes "$(xdotool search --name 'Select' >/dev/null 2>&1 && echo yes || echo "no: $(xdotool search --name . getwindowname %@ 2>/dev/null | sort -u | tr '\n' '|')")"
import -window root /out/wine-print-screen.png
xdotool key Escape
wineserver -k 2>/dev/null; sleep 2

# 9. The editor over a capture (fake backend), as a picture.
OBTURA_FAKE_CAPTURE="$(win /src/tests/fixtures/sample.png)" timeout 300 wine "$BIN" --screen >/dev/null 2>&1 &
sleep 20
import -window root /out/wine-editor.png
check "the editor opened" yes "$(xdotool search --name '^Obtura: ' >/dev/null 2>&1 && echo yes || echo no)"
# 9b. Format › More formatting › Line colour: one double-click on a colour in the chooser
#     picks it, and the Formatting panel comes back showing it. (A panel left open under
#     the chooser used to take the first click to close itself: the colour needed a second
#     click and the chooser stayed alone on the screen.) The places are those of the editor
#     on this 1280 x 800 screen; the colour is GTK's green 3 (#33d17a), in the chooser's
#     second column, third row.
xdotool mousemove 71 47 click 1; sleep 2
xdotool mousemove 220 354 click 1; sleep 2
xdotool mousemove 404 398 click 1; sleep 3
CHOOSER=$(xdotool search --name 'Pick a Color' 2>/dev/null | head -1)
check "the colour chooser opened" yes "$([ -n "$CHOOSER" ] && echo yes || echo no)"
if [ -n "$CHOOSER" ]; then
  eval "$(xdotool getwindowgeometry --shell "$CHOOSER" 2>/dev/null)"
  xdotool mousemove $((X + 111)) $((Y + 154)) click --repeat 2 --delay 80 1
  sleep 3
  check "one double-click in the chooser picks the colour" yes "$(xdotool search --name 'Pick a Color' >/dev/null 2>&1 && echo 'no: the chooser is still open' || echo yes)"
  import -window root /out/wine-format.png
  check "the Formatting panel came back with that colour" yes "$(convert /out/wine-format.png -crop 70x30+360+383 -format %c histogram:info:- 2>/dev/null | grep -qi '#33D17A' && echo yes || echo no)"
fi
wineserver -k 2>/dev/null
# 10. The installer: silent install for this user, the program runs from there, then the
#     uninstaller removes exactly what was installed (a file of the user's stays).
SETUP=$(ls /out/Obtura-*-setup.exe | tail -1)
timeout 300 wine "$SETUP" /S >/dev/null 2>&1
APPS=$(winepath -u "$(wine cmd /c echo %LOCALAPPDATA% 2>/dev/null | tr -d '\r')" 2>/dev/null)
INST="$APPS/Programs/Obtura"
check "the installer put obtura.exe in AppData\\Local\\Programs\\Obtura" yes "$([ -f "$INST/bin/obtura.exe" ] && echo yes || echo "no ($INST)")"
OUT=$(timeout 60 wine "$INST/bin/obtura.exe" --version 2>/dev/null | tr -d '\r')
check "the installed program runs" "obtura $VERSION" "$OUT"
check "a Start menu entry" yes "$(find "$WINEPREFIX/drive_c" -iname 'Obtura.lnk' -path '*Start Menu*' | grep -q . && echo yes || echo no)"
check "listed in Apps (uninstall entry)" yes "$(wine reg query 'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Obtura' /v DisplayVersion 2>/dev/null | grep -q "$VERSION" && echo yes || echo no)"
echo "keep me" > "$INST/my-notes.txt"
timeout 300 wine "$INST/uninstall.exe" /S >/dev/null 2>&1
# The uninstaller copies itself elsewhere and goes on from there: wait until it has removed
# itself (its last file), up to two minutes.
for _ in $(seq 120); do [ -e "$INST/uninstall.exe" ] || break; sleep 1; done
sleep 1
check "the uninstaller removed the program" yes "$([ ! -e "$INST/bin" ] && echo yes || echo "no: $(ls "$INST" | tr '\n' ' ')")"
check "a file of the user's in that folder stays" yes "$([ -f "$INST/my-notes.txt" ] && echo yes || echo no)"
wineserver -k 2>/dev/null

LOG=$(find "$WINEPREFIX" -name obtura.log 2>/dev/null | head -1)
if [ -n "$LOG" ]; then cp "$LOG" /out/wine-obtura.log; fi
if [ -n "${HOST_UID:-}" ]; then chown "$HOST_UID:$HOST_GID" /out/wine-* 2>/dev/null; fi
[ $FAILED = 0 ] && echo "All Wine checks passed." || echo "Some Wine checks FAILED."
exit $FAILED
