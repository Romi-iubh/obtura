# Making a release

Step by step, for version `X.Y.Z`. Everything runs on an Ubuntu machine with Docker,
Google Chrome (for the guide PDF and the demo pages), pandoc, Xvfb, xdotool,
ImageMagick and tesseract.

1. **Version.** Set `version = "X.Y.Z"` in the root `Cargo.toml` and run
   `scripts/check.sh` (it updates `Cargo.lock`).
2. **Release notes**, in three places that must agree:
   - `data/com.grasshillsco.Obtura.metainfo.xml`: a new `<release version="X.Y.Z"
     date="YYYY-MM-DD">` at the top of `<releases>`, with a `<description>`. The About
     window's *What's New* shows this text (a test checks that this version has one).
   - `packaging/changelog`: a new entry at the top (Debian format; the date line is
     `date -R`).
   - `CHANGELOG.md`.
   Also update the version in `data/obtura.1`, in `docs/user-guide.md` (title line and
   the install command), in `README.md` (install command), and the screenshot URLs in
   the metainfo file (they point at the `vX.Y.Z` tag). Check the metainfo file with
   `appstreamcli validate --no-net data/com.grasshillsco.Obtura.metainfo.xml` and the
   desktop file with `desktop-file-validate data/com.grasshillsco.Obtura.desktop`.
3. **Licences**, if `Cargo.lock` changed: `/usr/bin/python3 packaging/make-licenses.py`.
4. **Screenshots**, if the windows changed: `docs/make-screenshots.sh`. Look at every
   picture in `docs/screenshots/` before going on.
5. **User guide**, if it or the screenshots changed: `docs/make-guide.sh`.
6. **Tests:** `scripts/check.sh`, `scripts/test.sh`, and every smoke mode:
   `for m in screen region edit annotate phase2 phase3 cli record; do scripts/smoke.sh tests/fixtures/sample.png $m || break; done`.
7. **Package:** `packaging/build-deb.sh` (builds and runs every test on Ubuntu 24.04 in
   Docker, writes `dist/obtura_X.Y.Z-1_amd64.deb`). Check it:
   `packaging/test-install.sh dist/obtura_X.Y.Z-1_amd64.deb` installs it in clean
   Ubuntu 24.04 and 26.04 containers and tries it there (capture, text reading,
   recording, D-Bus activation, the dock menu's actions).
8. **Source archive and checksums:**
   ```
   git archive --format=tar.gz --prefix=obtura-X.Y.Z/ -o dist/obtura-X.Y.Z.tar.gz HEAD
   cp docs/Obtura-User-Guide.pdf dist/
   (cd dist && sha256sum obtura_X.Y.Z-1_amd64.deb obtura-X.Y.Z.tar.gz Obtura-User-Guide.pdf > SHA256SUMS)
   ```
9. **Publish:** commit, tag and push, then create the release with the four files:
   ```
   git commit -am "Obtura X.Y.Z" && git tag -a vX.Y.Z -m "Obtura X.Y.Z"
   git push origin main vX.Y.Z
   gh release create vX.Y.Z --title "Obtura X.Y.Z" --notes-file <notes.md> \
     dist/obtura_X.Y.Z-1_amd64.deb dist/obtura-X.Y.Z.tar.gz dist/Obtura-User-Guide.pdf dist/SHA256SUMS
   ```

Users update by installing the new `.deb` over the old one; their settings, library and
saved pictures are kept.
