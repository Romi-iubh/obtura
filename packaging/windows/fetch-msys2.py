#!/usr/bin/env python3
"""Downloads MSYS2 CLANG64 packages and everything they need, and unpacks them.

Usage: fetch-msys2.py DEST NAME...    (NAME without the "mingw-w64-clang-x86_64-" prefix)

Reads MSYS2's package list (clang64.db), follows each package's dependencies, downloads
every package once, checks it against the list's SHA-256 sum and unpacks it into DEST
(so DEST/clang64/bin holds the DLLs). DEST/msys2-packages.txt lists what was taken, with
versions and licences, and DEST/msys2-files/NAME.txt the files of each package, for the
licence notes of the Windows build.
"""
import hashlib
import io
import os
import re
import subprocess
import sys
import tarfile
import urllib.request

REPO = "https://repo.msys2.org/mingw/clang64/"
PREFIX = "mingw-w64-clang-x86_64-"


def fetch(url):
    with urllib.request.urlopen(url, timeout=120) as r:
        return r.read()


def zstd(data):
    return subprocess.run(["zstd", "-dc"], input=data, stdout=subprocess.PIPE, check=True).stdout


def read_db():
    packages, provides = {}, {}
    with tarfile.open(fileobj=io.BytesIO(zstd(fetch(REPO + "clang64.db")))) as tar:
        for member in tar.getmembers():
            if not member.name.endswith("/desc"):
                continue
            fields, key = {}, None
            for line in tar.extractfile(member).read().decode().splitlines():
                if line.startswith("%") and line.endswith("%"):
                    key = line.strip("%")
                    fields[key] = []
                elif line and key:
                    fields[key].append(line)
            name = fields["NAME"][0]
            packages[name] = fields
            for p in fields.get("PROVIDES", []):
                provides.setdefault(re.split(r"[<>=]", p)[0], name)
    return packages, provides


def main():
    dest, wanted = sys.argv[1], [PREFIX + n for n in sys.argv[2:]]
    packages, provides = read_db()
    chosen, todo = [], list(wanted)
    while todo:
        asked = re.split(r"[<>=]", todo.pop())[0]
        name = asked if asked in packages else provides.get(asked)
        if name is None:
            sys.exit(f"no MSYS2 package provides {asked}")
        if name in chosen:
            continue
        chosen.append(name)
        todo.extend(packages[name].get("DEPENDS", []))
    os.makedirs(os.path.join(dest, "msys2-files"), exist_ok=True)
    listing = []
    for name in sorted(chosen):
        p = packages[name]
        data = fetch(REPO + p["FILENAME"][0])
        if hashlib.sha256(data).hexdigest() != p["SHA256SUM"][0]:
            sys.exit(f"{p['FILENAME'][0]}: SHA-256 sum does not match MSYS2's list")
        with tarfile.open(fileobj=io.BytesIO(zstd(data))) as tar:
            members = [m for m in tar.getmembers() if not m.name.startswith(".")]
            tar.extractall(dest, members=members, filter="tar")
        short = name[len(PREFIX):]
        with open(os.path.join(dest, "msys2-files", short + ".txt"), "w") as f:
            f.write("".join(m.name + "\n" for m in members if m.isfile()))
        listing.append(f"{short} {p['VERSION'][0]} {' '.join(p.get('LICENSE', []))}")
        print(f"{name} {p['VERSION'][0]}", flush=True)
    with open(os.path.join(dest, "msys2-packages.txt"), "w") as f:
        f.write("\n".join(listing) + "\n")


if __name__ == "__main__":
    main()
