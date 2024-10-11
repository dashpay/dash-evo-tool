FROM ghcr.io/cross-rs/aarch64-unknown-linux-gnu:main

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

# Install protoc
RUN curl -Lo /tmp/protoc.zip \
      https://github.com/protocolbuffers/protobuf/releases/download/v25.2/protoc-25.2-linux-aarch_64.zip && \
    unzip /tmp/protoc.zip -d /usr/local/bin/ && \
    rm /tmp/protoc.zip


ENV SYSROOT=/usr/aarch64-linux-gnu

ENV CC_aarch64_unknown_linux_gnu="aarch64-linux-gnu-gcc-9 --sysroot=$SYSROOT"
ENV CXX_aarch64_unknown_linux_gnu="aarch64-linux-gnu-g++-9 --sysroot=$SYSROOT"
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="aarch64-linux-gnu-gcc-9 --sysroot=$SYSROOT"
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_AR=aarch64-linux-gnu-ar

ENV CFLAGS_aarch64_unknown_linux_gnu="--sysroot=$SYSROOT"
ENV CXXFLAGS_aarch64_unknown_linux_gnu="--sysroot=$SYSROOT -nostdinc++ -isystem $SYSROOT/include/c++/9 -isystem $SYSROOT/include/c++/9/aarch64-linux-gnu"
ENV CPPFLAGS_aarch64_unknown_linux_gnu="--sysroot=$SYSROOT"
ENV LDFLAGS_aarch64_unknown_linux_gnu="--sysroot=$SYSROOT"

ENV CXXFLAGS_aarch64_unknown_linux_gnu="--sysroot=$SYSROOT -nostdinc++ -isystem $SYSROOT/include/c++/9 -isystem $SYSROOT/include/c++/9/aarch64-linux-gnu"

ENV C_INCLUDE_PATH=""
ENV CPLUS_INCLUDE_PATH=""
ENV LIBRARY_PATH=""

# Set PROTOC environment variable
ENV PROTOC=/usr/local/bin/protoc

RUN aarch64-linux-gnu-g++ --sysroot=$SYSROOT -v -E - </dev/null