FROM ghcr.io/cross-rs/aarch64-unknown-linux-gnu:latest

RUN apt-get update && \
    apt-get install -y \
        clang \
        gcc-aarch64-linux-gnu \
        g++-aarch64-linux-gnu \
        libstdc++-5-dev-arm64-cross \
        libc6-dev-arm64-cross \
        libssl-dev \
        pkg-config \
        unzip \
        curl \
        build-essential \
        glibc-devel

# Install protoc
RUN curl -Lo /tmp/protoc.zip \
      https://github.com/protocolbuffers/protobuf/releases/download/v25.2/protoc-25.2-linux-aarch_64.zip && \
    unzip /tmp/protoc.zip -d /usr/local && \
    rm /tmp/protoc.zip

ENV CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
ENV CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_AR=aarch64-linux-gnu-ar

ENV SYSROOT=/usr/aarch64-linux-gnu
ENV CFLAGS_aarch64_unknown_linux_gnu="--sysroot=$SYSROOT"
ENV CPPFLAGS_aarch64_unknown_linux_gnu="--sysroot=$SYSROOT"
ENV LDFLAGS_aarch64_unknown_linux_gnu="--sysroot=$SYSROOT"

ENV C_INCLUDE_PATH=""
ENV CPLUS_INCLUDE_PATH=""
ENV LIBRARY_PATH=""

# Set PROTOC environment variable
ENV PROTOC=/usr/local/bin/protoc

RUN aarch64-linux-gnu-g++ --sysroot=$SYSROOT -v -E - </dev/null