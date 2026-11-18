#!/usr/bin/env bash
# Core crate tests, plus the pure logic in app/. Usage: scripts/test.sh [test-name-filter]
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/obtura-target}"
cargo test --offline \
  -p obtura-capture -p obtura-image -p obtura-scene -p obtura-render -p obtura-core -p obtura \
  -- "${1:-}" --nocapture
