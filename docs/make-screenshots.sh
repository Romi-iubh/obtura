#!/usr/bin/env bash
# Makes the screenshots of the user guide and the README (docs/screenshots/) by running
# Obtura on a virtual display with the fake capture backend, on the made-up pages of
# docs/demo/ (every name, number and address there is fictional). The README's main
# picture is framed by Obtura itself: its Beautify frame, saved with Ctrl+S.
#
# Usage: docs/make-screenshots.sh
# Needs: Xvfb, xdotool, ImageMagick, Google Chrome (draws the demo pages), tesseract.
# Settings, library and saved files go to a throwaway folder, never the owner's.
#
# The editor shots click at fixed places: the picture's top-left corner is at screen
# (199,130) in a 1280x800 display (editor 1115x713, an 800x500 picture centred), as in
# scripts/smoke.sh.
set -euo pipefail
SELF="$(realpath "$0")"
cd "$(dirname "$SELF")/.."
ROOT="$PWD"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/obtura-target}"
BIN="$CARGO_TARGET_DIR/debug/obtura"
DEMO="$ROOT/docs/demo"
SHOTS="$ROOT/docs/screenshots"

# ---------------------------------------------------------------- inside a virtual display
if [ "${1:-}" = inner ]; then
  shift
  SESSION="$1"
  shot() { /usr/bin/import -window root "$WORK/$1.png"; }
  # The window whose title matches $1, as it is on the screen.
  window_shot() {
    local id; id=$(xdotool search --name "$1" | tail -1)
    /usr/bin/import -window "$id" "$2"
  }
  act() { /usr/bin/gdbus call --session --dest com.grasshillsco.Obtura --object-path /com/grasshillsco/Obtura \
            --method org.freedesktop.Application.ActivateAction "$1" "[]" "{}" > /dev/null; }
  # Screen position of picture pixel (x,y) in the editor.
  P() { echo "$((199 + $1)) $((130 + $2))"; }
  drag() { xdotool mousemove $(P $1 $2) mousedown 1 mousemove $(P $(( ($1+$3)/2 )) $(( ($2+$4)/2 ))) mousemove $(P $3 $4) mouseup 1; sleep 0.3; }
  click() { xdotool mousemove $(P $1 $2) click 1; sleep 0.3; }
  case "$SESSION" in
    start)
      "$BIN" & APP=$!; trap "kill $APP 2>/dev/null" EXIT
      sleep 3; window_shot "^Obtura$" "$SHOTS/start.png"
      act record; sleep 5; window_shot "^Obtura$" "$SHOTS/record.png"
      xdotool key Escape; sleep 0.5
      act preferences; sleep 5; window_shot "^Obtura$" "$SHOTS/preferences.png"
      ;;
    overlay)
      "$BIN" --region & APP=$!; trap "kill $APP 2>/dev/null" EXIT
      sleep 3
      xdotool mousemove 230 150 mousedown 1 mousemove 600 400 mousemove 1050 600 mouseup 1
      sleep 1; /usr/bin/import -window root "$SHOTS/overlay.png"
      ;;
    editor)
      "$BIN" --screen & APP=$!; trap "kill $APP 2>/dev/null" EXIT
      sleep 4
      # The Line tool's Formatting panel: colour, thickness, line ends.
      xdotool key l f; sleep 1
      # The panel is a pop-up of its own: the screen, not the window, holds both.
      shot formatting
      convert "$WORK/formatting.png" -crop 700x640+0+0 +repage "$SHOTS/formatting.png"
      xdotool key Escape; sleep 0.5
      # Marks: highlighter, numbered steps, a pixelated card number with a note, a circle,
      # and a callout pointing at the button.
      xdotool key h; drag 216 138 314 138
      xdotool key n; click 26 178; click 26 258; click 26 400
      xdotool key x; drag 212 247 386 270
      xdotool key t; click 404 247; xdotool type --delay 30 "Hidden for good"; xdotool key Return; sleep 0.3
      xdotool key e; drag 206 323 352 355
      xdotool key k; drag 176 404 420 445; xdotool type --delay 30 "Then save"; xdotool key Return; sleep 0.3
      xdotool key Escape Escape; sleep 0.8
      window_shot "^Obtura: " "$SHOTS/editor.png"
      # Beautify, with the panel open: a gradient, a light window bar.
      xdotool key shift+b; sleep 1
      xdotool mousemove 919 92 click 1; sleep 0.5      # Frame around the picture
      xdotool mousemove 719 162 click 1; sleep 0.5     # Ocean
      xdotool mousemove 736 678 click 1; sleep 1       # Window bar: Light
      window_shot "^Obtura: " "$SHOTS/beautify.png"
      ;;
    redact)
      "$BIN" --screen & APP=$!; trap "kill $APP 2>/dev/null" EXIT
      sleep 4
      xdotool key ctrl+shift+x
      for i in $(seq 1 30); do sleep 0.5; done
      window_shot "^Obtura: " "$SHOTS/smart-redact.png"
      ;;
    library)
      # Three captures go to the library (--save, as a script does), then the window,
      # searched by a word that is only in the demo page.
      for p in "$DEMO/account.png" "$DEMO/desktop.png" "$ROOT/tests/fixtures/sample.png"; do
        OBTURA_FAKE_CAPTURE="$p" "$BIN" --screen --save "$WORK/lib-$(basename "$p")" > /dev/null
        sleep 1.2
      done
      "$BIN" --library & APP=$!; trap "kill $APP 2>/dev/null" EXIT
      sleep 8; window_shot "Library" "$SHOTS/library.png"
      ;;
    scrolling)
      # A scrolling capture over a long article (the fake screen scrolls through it): the
      # part that scrolls chosen, before Enter.
      OBTURA_FAKE_SCROLL="$DEMO/article.png" "$BIN" --scroll & APP=$!; trap "kill $APP 2>/dev/null" EXIT
      sleep 3
      xdotool mousemove 250 110 mousedown 1 mousemove 600 400 mousemove 1030 740 mouseup 1
      sleep 1; /usr/bin/import -window root "$SHOTS/scrolling.png"
      ;;
    frame)
      # $2 framed by Obtura's "my frame" (settings written by the caller), saved with Ctrl+S.
      OBTURA_FAKE_CAPTURE="$2" "$BIN" --screen & APP=$!; trap "kill $APP 2>/dev/null" EXIT
      sleep 4; xdotool key ctrl+s; sleep 2
      mv "$(ls "$HOME"/Pictures/Screenshots/Screenshot*.png | head -1)" "$3"
      ;;
  esac
  exit 0
fi

# ---------------------------------------------------------------- outside
cargo build -p obtura --offline
mkdir -p "$SHOTS"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
export WORK BIN DEMO SHOTS ROOT

# The demo pictures, drawn by Chrome from the HTML pages.
CHROME="$(command -v google-chrome || command -v chromium || command -v chromium-browser)"
page() {
  "$CHROME" --headless=new --disable-gpu --no-first-run --user-data-dir="$WORK/chrome" --hide-scrollbars \
    --window-size="$2" --screenshot="$DEMO/$1.png" "file://$DEMO/$1.html" 2>/dev/null
}
page account 800,500
page desktop 1280,800
page article 1280,3000
convert "$DEMO/article.png" -crop 1280x800+0+0 +repage "$WORK/article-top.png"

# GNOME settings in memory: the pictures never depend on (or touch) this desktop's shortcuts.
export GTK_A11Y=none GDK_BACKEND=x11 GSETTINGS_BACKEND=memory
unset WAYLAND_DISPLAY
run() { # run SESSION [screen size] [fake picture] [args...]
  local session="$1" size="$2" fake="$3"; shift 3
  local sandbox="$WORK/home-$session"
  rm -rf "$sandbox"; mkdir -p "$sandbox/config/obtura"
  [ -n "${SETTINGS:-}" ] && echo "$SETTINGS" > "$sandbox/config/obtura/settings.json"
  HOME="$sandbox" XDG_CONFIG_HOME="$sandbox/config" XDG_DATA_HOME="$sandbox/data" XDG_STATE_HOME="$sandbox/state" \
    XDG_CACHE_HOME="$sandbox/cache" OBTURA_FAKE_CAPTURE="$fake" \
    xvfb-run -a -s "-screen 0 ${size}x24" dbus-run-session -- bash "$SELF" inner "$session" "$@" > "$WORK/$session.log" 2>&1 \
    || { echo "session $session failed:"; tail -20 "$WORK/$session.log"; exit 1; }
  echo "made: $session"
}
# Preferences shows the save folders: a plain one rather than the throwaway folder.
SETTINGS='{"save_dir": "/home/you/Pictures/Screenshots", "record": {"dir": "/home/you/Videos/Screencasts"}}' \
  run start 1280x800 "$DEMO/account.png"
run overlay 1280x800 "$DEMO/desktop.png"
run editor 1280x800 "$DEMO/account.png"
run redact 1280x800 "$DEMO/account.png"
run library 1280x800 "$DEMO/account.png"
run scrolling 1280x800 "$WORK/article-top.png"
# The README's main picture: the editor, framed by Obtura (Ocean gradient, shadow).
SETTINGS='{"copy_after_capture": false, "auto_frame": true, "frame": {"background": {"Gradient": [{"r": 33, "g": 147, "b": 176, "a": 255}, {"r": 109, "g": 213, "b": 237, "a": 255}]}, "padding": 56.0, "corner": 12.0, "shadow": 28.0, "border": 0.0, "bar": "None", "torn": false, "ratio": null}}' \
  run frame 1920x1200 "$DEMO/account.png" "$SHOTS/editor.png" "$SHOTS/hero.png"
for f in "$SHOTS"/*.png; do printf "  %-40s %s\n" "${f#$ROOT/}" "$(identify -format "%wx%h" "$f")"; done
