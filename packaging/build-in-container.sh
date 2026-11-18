#!/usr/bin/env bash
# Runs inside the packaging/Dockerfile image (see packaging/build-deb.sh).
# Builds Obtura in release mode, runs the tests, and writes the .deb to /out.
set -euo pipefail
cd /src
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/target}"

echo "==> Building Obtura $(grep -m1 '^version' Cargo.toml | cut -d'"' -f2) on $(. /etc/os-release; echo "$PRETTY_NAME")"
cargo build --release --locked -p obtura

if [ "${SKIP_TESTS:-0}" != "1" ]; then
  echo "==> Tests (GStreamer $(gst-launch-1.0 --version | sed -n 's/.*version //p' | head -1), $(tesseract --version 2>&1 | head -1))"
  # The recording tests write their test files to the temporary folder.
  cargo test --release --locked --workspace
fi

echo "==> Packaging"
cargo deb -p obtura --no-build --locked
mkdir -p /out
rm -f /out/obtura_*.deb
cp "$CARGO_TARGET_DIR"/debian/obtura_*.deb /out/
# Files written from the container belong to the person who ran the build.
if [ -n "${HOST_UID:-}" ]; then chown -R "$HOST_UID:${HOST_GID:-$HOST_UID}" /out; fi
ls -l /out/*.deb
