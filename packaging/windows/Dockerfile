# Build environment for the Windows version of Obtura (cross-compiled on Linux).
#
# Windows GTK 4 and libadwaita come from MSYS2's CLANG64 packages (UCRT, clang, libc++),
# so Obtura is built for Rust's x86_64-pc-windows-gnullvm target with llvm-mingw, the
# same kind of toolchain MSYS2 builds them with. NSIS makes the installer; Wine runs the
# result for the tests. Used by packaging/windows/build-exe.sh.
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      curl ca-certificates xz-utils zstd python3 pkgconf file make git gcc libc6-dev \
      libglib2.0-bin librsvg2-bin imagemagick nsis zip unzip \
      wine wine64 xvfb xauth xdotool \
 && rm -rf /var/lib/apt/lists/*

# llvm-mingw: clang, lld and the mingw-w64 headers and libraries, UCRT by default.
ARG LLVM_MINGW=20261117
RUN curl -sSfL "https://github.com/mstorsjo/llvm-mingw/releases/download/${LLVM_MINGW}/llvm-mingw-${LLVM_MINGW}-ucrt-ubuntu-22.04-x86_64.tar.xz" \
      | tar -xJ -C /opt \
 && mv /opt/llvm-mingw-${LLVM_MINGW}-ucrt-ubuntu-22.04-x86_64 /opt/llvm-mingw

# Rust, pinned like the .deb build.
ARG RUST_VERSION=1.99.1
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/opt/llvm-mingw/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain ${RUST_VERSION} \
      --target x86_64-pc-windows-gnullvm \
 && rm -rf /opt/cargo/registry /opt/cargo/git

# GTK 4, libadwaita, librsvg (SVG icons), Tesseract (text reading) and everything they need,
# from MSYS2 (each package checked against the SHA-256 sum in MSYS2's package list).
COPY fetch-msys2.py /opt/fetch-msys2.py
ARG MSYS2_PACKAGES="gtk4 libadwaita librsvg tesseract-ocr tesseract-data-eng hicolor-icon-theme"
RUN python3 /opt/fetch-msys2.py /msys2 ${MSYS2_PACKAGES}

ENV PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_SYSROOT_DIR=/msys2 \
    PKG_CONFIG_LIBDIR=/msys2/clang64/lib/pkgconfig:/msys2/clang64/share/pkgconfig \
    CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER=x86_64-w64-mingw32-clang \
    WINEDEBUG=-all

WORKDIR /src
