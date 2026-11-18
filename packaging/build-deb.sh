#!/usr/bin/env bash
# Builds the Obtura .deb on Ubuntu 24.04 inside Docker, so that the package installs
# on Ubuntu 24.04 LTS and every newer release. The result goes to dist/.
#
# Usage: packaging/build-deb.sh            (build, test, package)
#        SKIP_TESTS=1 packaging/build-deb.sh
#
# The user guide PDF must be built first (docs/make-guide.sh): the package installs it.
# The first run downloads Ubuntu 24.04, Rust and the crates (5-15 minutes); later runs
# reuse the Docker volumes obtura-cargo and obtura-target.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"
IMAGE="obtura-build:24.04"

test -f docs/Obtura-User-Guide.pdf || { echo "docs/Obtura-User-Guide.pdf is missing: run docs/make-guide.sh first" >&2; exit 1; }
docker build -t "$IMAGE" packaging
mkdir -p dist
docker run --rm \
  -v "$ROOT":/src:ro \
  -v "$ROOT/dist":/out \
  -v obtura-cargo:/opt/cargo/registry \
  -v obtura-target:/target \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  -e SKIP_TESTS="${SKIP_TESTS:-0}" \
  "$IMAGE" bash /src/packaging/build-in-container.sh
