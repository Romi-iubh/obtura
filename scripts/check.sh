#!/usr/bin/env bash
# Fast compile check of the whole workspace.
set -euo pipefail
cd "$(dirname "$0")/.."
# Keep the target dir outside the project: the project path contains spaces.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/obtura-target}"
cargo check --workspace --all-targets --offline
