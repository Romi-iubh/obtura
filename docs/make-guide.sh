#!/usr/bin/env bash
# Builds docs/Obtura-User-Guide.pdf from docs/user-guide.md (the PDF that the User Guide
# item, F1, opens; installed with the package). Needs pandoc and Google Chrome or
# Chromium. Run it after changing the guide or its screenshots:   docs/make-guide.sh
# (docs/make-screenshots.sh makes the screenshots.)
set -euo pipefail
cd "$(dirname "$0")"
CHROME="$(command -v google-chrome || command -v chromium || command -v chromium-browser)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# The cover replaces the Markdown title and version line.
VERSION="$(grep -m1 '^version' ../Cargo.toml | cut -d'"' -f2)"
LOGO="$(realpath ../data/icons/hicolor/scalable/apps/com.grasshillsco.Obtura.svg)"
tail -n +4 user-guide.md > "$WORK/body.md"
cat > "$WORK/cover.html" <<HTML
<div class="cover">
<img class="logo" src="file://$LOGO" alt="">
<div class="title">Obtura User Guide</div>
<div class="version">Version $VERSION · for Ubuntu 24.04 LTS and newer</div>
</div>
HTML

pandoc -f gfm -t html5 -s \
  --metadata title="Obtura User Guide" --metadata lang=en \
  --css guide.css --include-before-body "$WORK/cover.html" \
  -V pagetitle="Obtura User Guide" \
  -o guide.html "$WORK/body.md"
# pandoc puts the title in a header block of its own; the cover has it already.
sed -i '/<header id="title-block-header">/,/<\/header>/d' guide.html

"$CHROME" --headless=new --disable-gpu --no-first-run --user-data-dir="$WORK/chrome" \
  --no-pdf-header-footer --generate-pdf-document-outline \
  --print-to-pdf="$PWD/Obtura-User-Guide.pdf" "file://$PWD/guide.html" 2>/dev/null
rm -f guide.html
echo "Wrote docs/Obtura-User-Guide.pdf"
