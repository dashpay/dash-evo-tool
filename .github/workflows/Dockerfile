FROM ghcr.io/cross-rs/aarch64-unknown-linux-gnu:latest

RUN apt-get update && \
    apt-get install -y \
        clang \
        gcc-aarch64-linux-gnu \
        g++-aarch64-linux-gnu \
        libstdc++-9-dev-arm64-cross \
        libc6-dev-arm64-cross \
        libssl-dev \
        pkg-config \
        unzip \
        curl \
        build-essential

# Install protoc (x86_64 version)
RUN curl -Lo /tmp/protoc.zip \
      https://github.com/protocolbuffers/protobuf/releases/download/v25.2/protoc-25.2-linux-x86_64.zip && \
    unzip /tmp/protoc.zip -d /usr/local && \
    rm /tmp/protoc.zip

# Ensure /usr/local/bin is in PATH
ENV PATH="/usr/local/bin:${PATH}"

# Set environment variables
ENV CC_aarch64_unknown_linux_gnu="aarch64-linux-gnu-gcc-9"
ENV CXX_aarch64_unknown_linux_gnu="aarch64-linux-gnu-g++-9"
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="aarch64-linux-gnu-gcc-9"
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_AR="aarch64-linux-gnu-ar"

ENV CFLAGS_aarch64_unknown_linux_gnu="--sysroot=/usr/aarch64-linux-gnu"
ENV CXXFLAGS_aarch64_unknown_linux_gnu="--sysroot=/usr/aarch64-linux-gnu -nostdinc++ -isystem /usr/aarch64-linux-gnu/include/c++/9 -isystem /usr/aarch64-linux-gnu/include/c++/9/aarch64-linux-gnu"
ENV LDFLAGS_aarch64_unknown_linux_gnu="--sysroot=/usr/aarch64-linux-gnu"

# Unset conflicting environment variables
ENV C_INCLUDE_PATH=""
ENV CPLUS_INCLUDE_PATH=""
ENV LIBRARY_PATH=""

# Set PROTOC environment variable
ENV PROTOC=/usr/local/bin/protoc
