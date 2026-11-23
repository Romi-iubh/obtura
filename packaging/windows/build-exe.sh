#!/usr/bin/env bash
# Builds Obtura for Windows (64-bit, Windows 10 and 11) inside Docker, cross-compiled on
# Linux. The results go to dist/windows/:
#   Obtura-<version>-setup.exe          the installer (for the current user, no admin rights)
#   Obtura-<version>-windows-x64.zip    the same files, to run without installing
#   SHA256SUMS-windows
#
# Usage: packaging/windows/build-exe.sh
#
# The user guide PDF must be built first (docs/make-guide.sh): it is installed with Obtura.
# The first run downloads Ubuntu, llvm-mingw, Rust, MSYS2's GTK packages and the crates
# (10-20 minutes); later runs reuse the image and the Docker volumes obtura-cargo and
# obtura-win-target. packaging/windows/test-exe.sh runs the result under Wine.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"
IMAGE="obtura-windows-build"

test -f docs/Obtura-User-Guide.pdf || { echo "docs/Obtura-User-Guide.pdf is missing: run docs/make-guide.sh first" >&2; exit 1; }
docker build -t "$IMAGE" packaging/windows
mkdir -p dist/windows
docker run --rm \
  -v "$ROOT":/src:ro \
  -v "$ROOT/dist/windows":/out \
  -v obtura-cargo:/opt/cargo/registry \
  -v obtura-win-target:/target \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  "$IMAGE" bash /src/packaging/windows/build-in-container.sh
