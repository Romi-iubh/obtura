# Build environment for the Obtura .deb.
#
# The package is built on the oldest Ubuntu release Obtura supports (24.04 LTS:
# GTK 4.14, libadwaita 1.5, GStreamer 1.24), so the same .deb installs on 24.04 and
# every newer release (25.04, 25.10, 26.04 LTS, ...). Used by packaging/build-deb.sh.
# The first two steps are the same as Foliox's, so Docker can reuse them.
ARG UBUNTU=24.04
FROM ubuntu:${UBUNTU}

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential pkg-config clang libclang-dev make curl ca-certificates git \
      libgtk-4-dev libadwaita-1-dev librsvg2-common dpkg-dev file xz-utils \
 && rm -rf /var/lib/apt/lists/*

# Rust, pinned so that a release can be rebuilt the same way.
ARG RUST_VERSION=1.99.1
ARG CARGO_DEB_VERSION=3.8.0
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain ${RUST_VERSION} \
 && cargo install cargo-deb --version ${CARGO_DEB_VERSION} --locked \
 && rm -rf /opt/cargo/registry /opt/cargo/git

# The programs Obtura runs, so that the tests using them run on 24.04 too: GStreamer
# with every encoder Obtura can use (recording, joining, trimming), tesseract (text
# reading), gst-discoverer-1.0 (the recording test checks its files with it), and fonts
# for the drawing tests.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      gstreamer1.0-tools gstreamer1.0-pipewire gstreamer1.0-plugins-base \
      gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly \
      gstreamer1.0-libav gstreamer1.0-plugins-base-apps tesseract-ocr tesseract-ocr-eng \
      fonts-dejavu-core fonts-noto-color-emoji \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
