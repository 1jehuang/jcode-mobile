FROM ubuntu:24.04

# Swift 6.4.1 and the rcodesign binary used by the Linux release setup require
# newer glibc than Amazon Linux 2023. Keep the host OS and its credentials intact.
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates clang cmake curl g++ git libcurl4 libedit2 libncurses6 \
    libsqlite3-0 libssl-dev libxml2 llvm make pkg-config python3 python3-cryptography \
    zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

ENV HOME=/build-home \
    PATH=/build-home/.local/bin:/build-home/.cargo/bin:/build-home/.local/share/swiftly/toolchains/6.4.1/usr/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    CARGO=justrust \
    JUSTRUST_REMOTE=off \
    CARGO_BUILD_JOBS=2
WORKDIR /workspace
# No upload command or network release side effect is part of this image.
CMD ["bash", "scripts/ios-bundle.sh", "--testflight"]
