#!/usr/bin/env bash
# Runs inside the obtura-windows-build container (see build-exe.sh): builds obtura.exe, puts
# it together with the GTK, libadwaita and Tesseract files it needs (from MSYS2), and makes
#   /out/Obtura-<version>-setup.exe          the installer (NSIS, for the current user)
#   /out/Obtura-<version>-windows-x64.zip    the same files, to run without installing
set -euo pipefail
SRC=/src
OUT=/out
TRIPLE=x86_64-pc-windows-gnullvm
M=/msys2/clang64
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$SRC/Cargo.toml" | head -1)
WORK=/work
STAGE=$WORK/Obtura
rm -rf "$WORK"
mkdir -p "$WORK" "$STAGE/bin" "$STAGE/lib" "$STAGE/share/doc/obtura" "$STAGE/share/obtura"

echo "== Obtura $VERSION for Windows"

# --- The program's icon (from the scalable app icon) ---
ICON_SVG="$SRC/data/icons/hicolor/scalable/apps/com.grasshillsco.Obtura.svg"
for s in 16 20 24 32 40 48 64 128 256; do
  rsvg-convert -w $s -h $s "$ICON_SVG" -o "$WORK/icon-$s.png"
done
convert "$WORK"/icon-{16,20,24,32,40,48,64,128,256}.png "$WORK/obtura.ico"

# --- Build ---
OBTURA_WINDOWS_ICON="$WORK/obtura.ico" cargo build --release --locked --target "$TRIPLE" --target-dir /target -p obtura
cp "/target/$TRIPLE/release/obtura.exe" "$STAGE/bin/"

# --- Programs and image loaders shipped beside it ---
cp "$M/bin/tesseract.exe" "$M/bin/gdbus.exe" "$STAGE/bin/"
LOADERS=lib/gdk-pixbuf-2.0/2.10.0/loaders
mkdir -p "$STAGE/$LOADERS"
cp "$M/$LOADERS"/*.dll "$STAGE/$LOADERS/"

# --- Every DLL these need, found in MSYS2's bin (the rest are Windows' own) ---
python3 - "$STAGE" "$M/bin" <<'PY'
import os, subprocess, sys
stage, bin_dir = sys.argv[1], sys.argv[2]
available = {f.lower(): f for f in os.listdir(bin_dir) if f.lower().endswith(".dll")}
todo = [os.path.join(r, f) for r, _, fs in os.walk(stage) for f in fs if f.lower().endswith((".exe", ".dll"))]
copied = set()
while todo:
    out = subprocess.run(["llvm-objdump", "-p", todo.pop()], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        line = line.strip()
        if not line.startswith("DLL Name:"):
            continue
        name = line.split(":", 1)[1].strip().lower()
        if name in available and name not in copied:
            copied.add(name)
            dst = os.path.join(stage, "bin", available[name])
            subprocess.run(["cp", os.path.join(bin_dir, available[name]), dst], check=True)
            todo.append(dst)
print(f"{len(copied)} DLLs")
PY

# The loaders' list, as gdk-pixbuf reads it: written by gdk-pixbuf's own tool under Wine,
# with the paths made relative to the installation folder.
export WINEPREFIX=$WORK/wine WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml="
(cd "$STAGE/bin" && GDK_PIXBUF_MODULEDIR='..\lib\gdk-pixbuf-2.0\2.10.0\loaders' wine "$M/bin/gdk-pixbuf-query-loaders.exe" 2>/dev/null) \
  | tr -d '\r' | sed -E 's#^"[^"]*(lib[\\/]+gdk-pixbuf-2\.0[\\/]+2\.10\.0[\\/]+loaders[\\/]+[^"\\/]+\.dll)"#"\1"#' \
  > "$STAGE/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
grep -q 'pixbufloader_svg\|pixbufloader-svg' "$STAGE/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache" \
  || { echo "loaders.cache has no SVG loader:" >&2; cat "$STAGE/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache" >&2; exit 1; }

# --- Data: settings schemas, icons, text-reading languages, fonts setup ---
mkdir -p "$STAGE/share/glib-2.0/schemas"
cp "$M"/share/glib-2.0/schemas/*.gschema.xml "$STAGE/share/glib-2.0/schemas/"
glib-compile-schemas "$STAGE/share/glib-2.0/schemas"
mkdir -p "$STAGE/share/icons"
cp -r "$M/share/icons/Adwaita" "$M/share/icons/hicolor" "$STAGE/share/icons/"
rm -f "$STAGE"/share/icons/*/icon-theme.cache
for kind in scalable symbolic; do
  mkdir -p "$STAGE/share/icons/hicolor/$kind/apps"
  cp "$SRC/data/icons/hicolor/$kind/apps/"*.svg "$STAGE/share/icons/hicolor/$kind/apps/"
done
cp -r "$M/share/tessdata" "$STAGE/share/"
if [ -d "$M/etc/fonts" ]; then
  mkdir -p "$STAGE/etc" && cp -r "$M/etc/fonts" "$STAGE/etc/"
fi
cp "$SRC/docs/Obtura-User-Guide.pdf" "$STAGE/share/obtura/"
cp "$WORK/obtura.ico" "$STAGE/share/obtura/"

# --- Licences: Obtura's, the Rust crates', and each MSYS2 package a shipped file came from ---
cp "$SRC/LICENSE" "$STAGE/share/doc/obtura/LICENSE.txt"
cp "$SRC/packaging/windows/THIRD-PARTY-LICENSES.txt" "$STAGE/share/doc/obtura/"
python3 - "$STAGE" /msys2 <<'PY'
import os, shutil, sys
stage, root = sys.argv[1], sys.argv[2]
shipped = set()
for r, _, fs in os.walk(stage):
    for f in fs:
        shipped.add(os.path.relpath(os.path.join(r, f), stage).replace(os.sep, "/").lower())
versions = {}
for line in open(os.path.join(root, "msys2-packages.txt")):
    name, version, *licence = line.split()
    versions[name] = (version, " ".join(licence) or "see its licence files")
used = []
for name in sorted(versions):
    files = [l.strip() for l in open(os.path.join(root, "msys2-files", name + ".txt"))]
    mine = [f for f in files if f.startswith("clang64/")]
    if not any(f[len("clang64/"):].lower() in shipped for f in mine):
        continue
    used.append(name)
    for f in mine:
        if f.startswith("clang64/share/licenses/"):
            dst = os.path.join(stage, f[len("clang64/"):])
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            shutil.copy(os.path.join(root, f), dst)
with open(os.path.join(stage, "share/doc/obtura/WINDOWS-COMPONENTS.txt"), "w", newline="\r\n") as out:
    out.write("Obtura for Windows: the components it comes with\n")
    out.write("=" * 78 + "\n\n")
    out.write("These libraries and programs are taken unchanged from the MSYS2 project's\n")
    out.write("CLANG64 packages (https://www.msys2.org). Each keeps its own licence; the texts\n")
    out.write("are in the folder share\\licenses. Their source code is published by MSYS2\n")
    out.write("(https://github.com/msys2/MINGW-packages, and the sources each package names)\n")
    out.write("under the same names and versions.\n\n")
    out.write(f"{'Package':34} {'Version':16} Licence\n")
    for name in used:
        v, l = versions[name]
        out.write(f"{name:34} {v:16} {l.replace('spdx:', '')}\n")
print(f"{len(used)} MSYS2 packages shipped")
PY

# --- The portable zip ---
cat > "$WORK/README.txt" <<EOF
Obtura $VERSION for Windows (64-bit, Windows 10 and 11)

To run it without installing: open the Obtura folder, then bin, and double-click
obtura.exe. Keep the folders together (bin, lib, share): Obtura needs them.

The installer (Obtura-$VERSION-setup.exe) does the same and adds Obtura to the Start menu.

Settings and the library are kept in %APPDATA%\\Obtura and %LOCALAPPDATA%\\Obtura.
Licences: Obtura\\share\\doc\\obtura.
EOF
sed -i 's/$/\r/' "$WORK/README.txt"
mkdir -p "$OUT"
ZIP="$OUT/Obtura-$VERSION-windows-x64.zip"
rm -f "$ZIP"
(cd "$WORK" && zip -qr9 "$ZIP" Obtura README.txt)

# --- The installer ---
# The uninstaller removes exactly the files installed (never anything else in the folder).
python3 - "$STAGE" "$WORK/files.nsh" "$WORK/unfiles.nsh" <<'PY'
import os, sys
stage, inst, uninst = sys.argv[1:4]
dirs, lines = [], []
for r, ds, fs in os.walk(stage):
    rel = os.path.relpath(r, stage)
    win = "" if rel == "." else "\\" + rel.replace("/", "\\")
    dirs.append(win)
    if fs:
        lines.append(f'SetOutPath "$INSTDIR{win}"')
        lines += [f'File "{os.path.join(r, f)}"' for f in sorted(fs)]
with open(inst, "w") as f:
    f.write("\n".join(lines) + "\n")
with open(uninst, "w") as f:
    for r, ds, fs in os.walk(stage):
        rel = os.path.relpath(r, stage)
        win = "" if rel == "." else "\\" + rel.replace("/", "\\")
        for name in sorted(fs):
            f.write(f'Delete "$INSTDIR{win}\\{name}"\n')
    for win in sorted(dirs, key=len, reverse=True):
        if win:
            f.write(f'RMDir "$INSTDIR{win}"\n')
PY
SIZE_KB=$(du -sk "$STAGE" | cut -f1)
makensis -V2 -DVERSION="$VERSION" -DSIZE_KB="$SIZE_KB" -DFILES="$WORK/files.nsh" -DUNFILES="$WORK/unfiles.nsh" \
  -DICON="$WORK/obtura.ico" -DOUTFILE="$OUT/Obtura-$VERSION-setup.exe" "$SRC/packaging/windows/installer.nsi"

(cd "$OUT" && sha256sum "Obtura-$VERSION-setup.exe" "Obtura-$VERSION-windows-x64.zip" > "SHA256SUMS-windows")
if [ -n "${HOST_UID:-}" ]; then
  chown "$HOST_UID:$HOST_GID" "$OUT" "$OUT/Obtura-$VERSION-setup.exe" "$OUT/Obtura-$VERSION-windows-x64.zip" "$OUT/SHA256SUMS-windows"
fi
ls -la "$OUT"
echo "== $(du -sh "$STAGE" | cut -f1) installed"
