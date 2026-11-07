# What Oxbow's Linux packages are built in, locally and by the release workflow alike. Ubuntu 22.04's glibc is old
# enough that the result runs on most distributions still supported.
#
#   docker build -t oxbow-linux packaging/linux
#   docker run --rm -v "$PWD:/src:ro" -v "$PWD/dist:/out" oxbow-linux
FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl git pkg-config dpkg-dev file desktop-file-utils \
        libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev \
        libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
    && rm -rf /var/lib/apt/lists/*

ARG RUST_VERSION=1.98.1
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain ${RUST_VERSION}
RUN cargo install cargo-deb --locked --version 3.8.0

# appimagetool and the AppImage runtime, pinned and checked, so building never fetches anything else.
RUN curl -fsSL -o /usr/local/bin/appimagetool https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage \
    && echo "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0  /usr/local/bin/appimagetool" | sha256sum -c - \
    && chmod +x /usr/local/bin/appimagetool \
    && curl -fsSL -o /usr/local/share/appimage-runtime-x86_64 https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 \
    && echo "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d  /usr/local/share/appimage-runtime-x86_64" | sha256sum -c -
# No FUSE in a container: AppImages (appimagetool is one) unpack themselves instead.
ENV APPIMAGE_EXTRACT_AND_RUN=1 APPIMAGE_RUNTIME=/usr/local/share/appimage-runtime-x86_64

# The source is mounted read-only at /src; builds go to /build and packages to /out.
ENV CARGO_TARGET_DIR=/build/target OUT=/out
RUN git config --global user.name "Oxbow build" && git config --global user.email "build@localhost" \
    && git config --global --add safe.directory '*'
WORKDIR /src
CMD ["bash", "packaging/linux/build.sh"]
