#!/usr/bin/env bash
# Installs an Obtura .deb in clean Ubuntu containers, the way a user does (apt, with the
# recommended packages), then checks it there on a virtual display, with the fake capture
# backend (a test picture, never a real screen):
#   - obtura --version, and the files the package installs;
#   - a capture saved as a PNG from the command line (exit status, size);
#   - text reading (tesseract) of that picture, from the command line;
#   - a recording (the fake screen sharing's test picture) as MP4, stopped by running
#     obtura --record again;
#   - D-Bus activation, as the app grid and the dock use it: Activate opens the start
#     window, ActivateAction("library") the library, as the dock menu does.
#
# Usage: packaging/test-install.sh dist/obtura_X.Y.Z-1_amd64.deb [release ...]
#        (releases default to 24.04 and 26.04)
set -euo pipefail
cd "$(dirname "$0")/.."
DEB="$(realpath "${1:?usage: packaging/test-install.sh file.deb [release ...]}")"
shift
RELEASES=("$@")
[ ${#RELEASES[@]} -eq 0 ] && RELEASES=(24.04 26.04)
FIXTURE="$(realpath tests/fixtures/sample.png)"

for r in "${RELEASES[@]}"; do
  echo "==> Ubuntu $r"
  docker run --rm -v "$DEB":/tmp/obtura.deb:ro -v "$FIXTURE":/tmp/sample.png:ro "ubuntu:$r" bash -c '
    set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive
    # The minimal images skip manual pages and /usr/share/doc; a desktop does not.
    rm -f /etc/dpkg/dpkg.cfg.d/excludes
    apt-get update -qq
    apt-get install -y -qq /tmp/obtura.deb > /tmp/apt.log 2>&1 || { tail -30 /tmp/apt.log; exit 1; }
    apt-get install -y -qq xvfb xauth dbus-x11 xdotool procps file libglib2.0-bin > /dev/null 2>&1
    FAILED=0
    check() { if [ "$2" = "$3" ]; then echo "  ok: $1"; else echo "  FAIL: $1: wanted $2, got $3"; FAILED=1; fi; }
    obtura --version
    for p in /usr/bin/tesseract /usr/bin/gst-launch-1.0; do printf "  %-24s %s\n" "$p" "$(test -x $p && echo present || echo missing)"; done
    for e in pipewiresrc x264enc vp8enc webmmux mp4mux splitmuxsrc avwait pngenc videocrop; do
      printf "  %-24s %s\n" "GStreamer $e" "$(/usr/bin/gst-inspect-1.0 --exists $e && echo present || echo missing)"
    done
    for f in /usr/share/obtura/Obtura-User-Guide.pdf /usr/share/man/man1/obtura.1.gz /usr/share/doc/obtura/copyright \
             /usr/share/doc/obtura/THIRD-PARTY-LICENSES.txt /usr/share/applications/com.grasshillsco.Obtura.desktop \
             /usr/share/dbus-1/services/com.grasshillsco.Obtura.service /usr/share/metainfo/com.grasshillsco.Obtura.metainfo.xml \
             /usr/share/icons/hicolor/scalable/apps/com.grasshillsco.Obtura.svg \
             /usr/share/icons/hicolor/symbolic/apps/com.grasshillsco.Obtura-symbolic.svg; do
      check "installed $f" yes "$(test -s $f && echo yes || echo no)"
    done
    mkdir -p /root/.config/obtura
    echo "{\"record\": {\"format\": \"mp4\", \"countdown\": 0}}" > /root/.config/obtura/settings.json
    export OBTURA_FAKE_CAPTURE=/tmp/sample.png GTK_A11Y=none
    xvfb-run -a -s "-screen 0 1280x800x24" dbus-run-session -- bash -c "
      FAILED=0
      check() { if [ \"\$2\" = \"\$3\" ]; then echo \"  ok: \$1\"; else echo \"  FAIL: \$1: wanted \$2, got \$3\"; FAILED=1; fi; }
      obtura --screen --save /tmp/shot.png > /tmp/save.log 2>&1; check \"--save exit status\" 0 \$?
      check \"saved picture is 800x500\" yes \"\$(file /tmp/shot.png | grep -q \"800 x 500\" && echo yes || echo no)\"
      # --ocr keeps running (it holds the clipboard): it is started with & and not waited for.
      obtura --screen --ocr > /tmp/ocr.txt 2>/tmp/ocr.err &
      for i in \$(seq 1 40); do [ -s /tmp/ocr.txt ] && break; sleep 0.5; done
      check \"--ocr reads the text\" yes \"\$(grep -q fixture /tmp/ocr.txt && echo yes || echo \"no: \$(cat /tmp/ocr.txt /tmp/ocr.err)\")\"
      # The --ocr instance is the running one: --record goes to it, and so does the second --record.
      obtura --record --screen > /tmp/rec.txt 2>/tmp/rec.err &
      sleep 4; obtura --record; check \"second --record exit status\" 0 \$?
      for i in \$(seq 1 40); do [ -s /tmp/rec.txt ] && break; sleep 0.5; done
      F=\$(cat /tmp/rec.txt)
      check \"recording saved as MP4\" yes \"\$(case \"\$F\" in /root/Videos/Screencasts/Recording*.mp4) [ -s \"\$F\" ] && echo yes;; *) echo \"no: \$F \$(cat /tmp/rec.err)\";; esac)\"
      check \"the MP4 holds an H.264 picture\" yes \"\$(grep -aq avc1 \"\$F\" && echo yes || echo no)\"
      obtura --library; sleep 2
      check \"--library opens the library\" yes \"\$(xdotool search --name Library > /dev/null && echo yes || echo no)\"
      kill \$(pgrep -x obtura) 2>/dev/null; sleep 1
      # D-Bus activation, through the service file, with no Obtura running.
      gdbus call --session --dest com.grasshillsco.Obtura --object-path /com/grasshillsco/Obtura \
        --method org.freedesktop.Application.Activate \"{}\" > /dev/null; sleep 4
      check \"D-Bus activation opens the start window\" yes \"\$(xdotool search --name \"^Obtura\$\" > /dev/null && echo yes || echo no)\"
      gdbus call --session --dest com.grasshillsco.Obtura --object-path /com/grasshillsco/Obtura \
        --method org.freedesktop.Application.ActivateAction library \"[]\" \"{}\" > /dev/null; sleep 2
      check \"the dock menu action opens the library\" yes \"\$(xdotool search --name Library > /dev/null && echo yes || echo no)\"
      kill \$(pgrep -x obtura) 2>/dev/null
      exit \$FAILED" || FAILED=1
    exit $FAILED
  '
done
