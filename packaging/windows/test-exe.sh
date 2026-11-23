#!/usr/bin/env bash
# Runs the Windows build (dist/windows/Obtura-*-windows-x64.zip, from build-exe.sh) under
# Wine in the build container, on a virtual screen: see test-in-container.sh for what it
# checks. Pictures of the start window and the editor go to dist/windows/wine-*.png.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"
ls dist/windows/Obtura-*-windows-x64.zip >/dev/null || { echo "build it first: packaging/windows/build-exe.sh" >&2; exit 1; }
docker run --rm \
  -v "$ROOT":/src:ro \
  -v "$ROOT/dist/windows":/out \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  obtura-windows-build bash /src/packaging/windows/test-in-container.sh
