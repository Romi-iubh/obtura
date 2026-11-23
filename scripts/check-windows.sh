#!/usr/bin/env bash
# Fast compile check of the Windows version (cargo check for x86_64-pc-windows-gnullvm in
# the obtura-windows-build Docker image; the first run builds the image, see
# packaging/windows/build-exe.sh). Run it after changing code that Windows also compiles.
set -euo pipefail
cd "$(dirname "$0")/.."
docker image inspect obtura-windows-build >/dev/null 2>&1 || docker build -t obtura-windows-build packaging/windows
docker run --rm -v "$PWD":/src:ro -v obtura-cargo:/opt/cargo/registry -v obtura-win-target:/target \
  obtura-windows-build cargo check --locked --target x86_64-pc-windows-gnullvm --target-dir /target --workspace "$@"
