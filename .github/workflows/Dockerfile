FROM ghcr.io/cross-rs/aarch64-unknown-linux-gnu:latest

RUN apt-get update && \
    apt-get install -y libssl-dev pkg-config unzip curl clang libc6 libc6-dev gcc

# Install protoc
RUN curl -Lo /tmp/protoc.zip \
      https://github.com/protocolbuffers/protobuf/releases/download/v25.2/protoc-25.2-linux-aarch_64.zip && \
    unzip /tmp/protoc.zip -d /usr/local && \
    rm /tmp/protoc.zip

# Set PROTOC environment variable
ENV PROTOC=/usr/local/bin/protoc