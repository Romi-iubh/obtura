#!/usr/bin/env python3
"""Writes the licence files that ship with the Obtura package:

  packaging/copyright                  /usr/share/doc/obtura/copyright (Debian format)
  packaging/THIRD-PARTY-LICENSES.txt   /usr/share/doc/obtura/THIRD-PARTY-LICENSES.txt

Obtura's binary contains the Rust crates listed by `cargo tree`; their licences ask that
their notices go with the program. (GTK, cairo, Pango, GLib, GStreamer and Tesseract are
the system's own libraries and programs, not part of the binary.) Run this again whenever
Cargo.lock changes:

    /usr/bin/python3 packaging/make-licenses.py

It also writes packaging/windows/THIRD-PARTY-LICENSES.txt for the Windows build (the crates
linked into obtura.exe; GTK and the other libraries that build ships are listed by the
Windows build itself, in WINDOWS-COMPONENTS.txt).

It reads the crates' sources from the local Cargo registry (run `cargo fetch` first).
"""
import json
import pathlib
import re
import subprocess
import textwrap

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT_COPYRIGHT = ROOT / "packaging" / "copyright"
OUT_THIRD = ROOT / "packaging" / "THIRD-PARTY-LICENSES.txt"
OUT_THIRD_WINDOWS = ROOT / "packaging" / "windows" / "THIRD-PARTY-LICENSES.txt"
LINUX = "x86_64-unknown-linux-gnu"
WINDOWS = "x86_64-pc-windows-gnullvm"
WORKSPACE = {"obtura", "obtura-capture", "obtura-image", "obtura-scene", "obtura-render", "obtura-core"}
RULE = "=" * 78


def cargo(*args):
    return subprocess.run(["cargo", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout


def crates(target=LINUX):
    """(name, version, licence, licence files, authors) of every crate linked into obtura."""
    tree = cargo("tree", "-p", "obtura", "-e", "normal", "--prefix", "none", "--format", "{p}",
                 "--target", target, "--locked")
    wanted = set()
    for line in tree.splitlines():
        m = re.match(r"(\S+) v(\S+)", line.strip())
        if m and m.group(1) not in WORKSPACE:
            wanted.add(m.groups())
    meta = json.loads(cargo("metadata", "--format-version", "1", "--locked"))
    out = []
    for p in meta["packages"]:
        if (p["name"], p["version"]) not in wanted:
            continue
        folder = pathlib.Path(p["manifest_path"]).parent
        files = sorted(f for f in folder.iterdir()
                       if f.is_file() and re.match(r"(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|UNLICENSE)", f.name, re.I))
        out.append((p["name"], p["version"], p.get("license") or "see files", files, p.get("authors") or []))
    missing = wanted - {(n, v) for n, v, *_ in out}
    if missing:
        raise SystemExit(f"not in cargo metadata: {sorted(missing)}")
    return sorted(out)


def is_apache(text):
    return "Apache License" in text and "Version 2.0, January 2004" in text and len(text) > 8000


def is_mit(text):
    return "Permission is hereby granted, free of charge" in text


def chosen_texts(licence, files):
    """The licence texts that apply: for a crate offering MIT among others, MIT is the
    licence Obtura uses it under; the standard Apache text is printed once at the end."""
    texts = [(f.name, f.read_text(errors="replace").strip()) for f in files]
    if "MIT" in licence and any(is_mit(t) for _, t in texts):
        keep = [(n, t) for n, t in texts if is_mit(t)]
        # "(MIT OR Apache-2.0) AND Unicode-3.0": the licence after AND applies as well.
        if " AND " in licence:
            keep += [(n, t) for n, t in texts if not is_mit(t) and not is_apache(t)]
        return keep
    apache = "[The Apache License 2.0: its full text is at the end of this file.]"
    return [(n, apache) if is_apache(t) else (n, t) for n, t in texts]


def apache_text(crate_list):
    for _, _, _, files, _ in crate_list:
        for f in files:
            t = f.read_text(errors="replace").strip()
            if is_apache(t):
                return t
    raise SystemExit("no Apache License 2.0 text found among the crates")


LINUX_INTRO = """\
    Obtura is free software under the GNU Affero General Public License, version 3
    or later (see /usr/share/doc/obtura/copyright). Its source code is at
    https://github.com/Romi-iubh/obtura (every release also has a source archive).

    The Obtura program contains the Rust crates listed below. Each keeps its own
    copyright and licence, reproduced here as those licences require. The source
    code of every crate listed is published on https://crates.io under the same
    name and version.

    Obtura also uses libraries and programs of the system it is installed on (GTK,
    libadwaita, GLib, cairo, Pango, GStreamer, Tesseract); they are not part of the
    Obtura program and come with their own packages and licences.
    """

WINDOWS_INTRO = """\
    Obtura is free software under the GNU Affero General Public License, version 3
    or later (see LICENSE.txt in this folder). Its source code is at
    https://github.com/Romi-iubh/obtura (every release also has a source archive).

    The Obtura program (obtura.exe) contains the Rust crates listed below. Each keeps
    its own copyright and licence, reproduced here as those licences require. The
    source code of every crate listed is published on https://crates.io under the
    same name and version.

    Obtura for Windows also comes with GTK, libadwaita, GLib, cairo, Pango, Tesseract
    and the libraries they need, as separate files (DLLs and programs) taken unchanged
    from the MSYS2 project (https://www.msys2.org). WINDOWS-COMPONENTS.txt in this
    folder lists each one with its version and licence; their licence texts are in
    the folder share\\licenses, and their source code is published by MSYS2
    (https://github.com/msys2/MINGW-packages) under the same names and versions.
    """


def write_third_party(crate_list, out=OUT_THIRD, intro=LINUX_INTRO):
    parts = [f"Obtura: third-party software and licences\n{RULE}\n\n" + textwrap.dedent(intro)]
    parts.append(f"\n{RULE}\nRust crates\n{RULE}\n")
    # Crates whose licence texts are the same are listed together.
    groups = {}
    for name, version, licence, files, authors in crate_list:
        texts = chosen_texts(licence, files)
        if not texts:
            who = ", ".join(authors) or "its authors"
            texts = [("(no licence file in the crate)", f"Licensed under {licence}. Copyright {who}.")]
        key = "\n\n".join(t for _, t in texts)
        groups.setdefault(key, []).append(f"{name} {version} ({licence})")
    for key, names in sorted(groups.items(), key=lambda kv: kv[1][0]):
        parts.append("\n--- " + "; ".join(names) + " ---\n\n" + key + "\n")
    parts.append(f"\n{RULE}\nThe Apache License, Version 2.0\n{RULE}\n\n{apache_text(crate_list)}\n")
    out.write_text("".join(parts))


def write_copyright():
    """Debian machine-readable copyright file (DEP-5), with the AGPL's full text."""
    agpl = (ROOT / "LICENSE").read_text().strip("\n")
    body = "\n".join(" " + (l if l.strip() else ".") for l in agpl.splitlines())
    OUT_COPYRIGHT.write_text(textwrap.dedent("""\
        Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
        Upstream-Name: Obtura
        Upstream-Contact: https://github.com/Romi-iubh/obtura/issues
        Source: https://github.com/Romi-iubh/obtura
        Comment:
         The obtura binary also contains a number of Rust crates. Their notices and
         licence texts are in /usr/share/doc/obtura/THIRD-PARTY-LICENSES.txt.

        Files: *
        Copyright: 2026 Rosaire
        License: AGPL-3+

        Files: rust-crates/*
        Copyright: the authors of each crate
        License: MIT or Apache-2.0 and others
         Each crate's licence is in /usr/share/doc/obtura/THIRD-PARTY-LICENSES.txt.

        License: AGPL-3+
         This program is free software: you can redistribute it and/or modify it
         under the terms of the GNU Affero General Public License as published by
         the Free Software Foundation, either version 3 of the License, or (at your
         option) any later version.
         .
         This program is distributed in the hope that it will be useful, but
         WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY
         or FITNESS FOR A PARTICULAR PURPOSE. See the GNU Affero General Public
         License for more details.
         .
         The full text of the GNU Affero General Public License version 3 follows.
         .
        """) + body + "\n")


def main():
    crate_list = crates()
    write_third_party(crate_list)
    write_copyright()
    windows = crates(WINDOWS)
    write_third_party(windows, OUT_THIRD_WINDOWS, WINDOWS_INTRO)
    licences = sorted({l for _, _, l, _, _ in crate_list})
    print(f"{len(crate_list)} crates; licences: {', '.join(licences)}")
    print(f"wrote {OUT_THIRD.relative_to(ROOT)} and {OUT_COPYRIGHT.relative_to(ROOT)}")
    print(f"{len(windows)} crates for Windows; wrote {OUT_THIRD_WINDOWS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
