# Dash Evo Tool — Full E2E Test Environment
# Builds the Tauri app and runs WebdriverIO tests against it via tauri-driver.
#
# Usage:
#   docker compose -f docker/e2e/docker-compose.yml up --build
#
# Requires: Docker with BuildKit (default since Docker 23+)

FROM ubuntu:24.04 AS base

ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC

# System dependencies for Tauri (WebKit2GTK), Xvfb (virtual display), and build tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    # Build essentials
    build-essential \
    ca-certificates \
    clang \
    curl \
    wget \
    git \
    pkg-config \
    # Tauri / WebKit2GTK dependencies
    libwebkit2gtk-4.1-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    libssl-dev \
    libxdo-dev \
    # protobuf-compiler from apt is too old (3.x); protoc v25.2+ installed below
    # Virtual display for headless GUI testing
    xvfb \
    # WebKit2GTK WebDriver (for tauri-driver)
    webkit2gtk-driver \
    # Misc runtime deps
    libglib2.0-dev \
    libsoup-3.0-dev \
    libjavascriptcoregtk-4.1-dev \
    # dbus for desktop integration
    dbus-x11 \
    unzip \
    # sqlite3 CLI for test database seeding
    sqlite3 \
    # Clean up
    && rm -rf /var/lib/apt/lists/*

# ---- Install protoc v25.2 (dash-sdk requires v25.2+) ----
RUN ARCH=$(dpkg --print-architecture) && \
    if [ "$ARCH" = "arm64" ]; then PB_ARCH="aarch_64"; else PB_ARCH="x86_64"; fi && \
    curl -fsSL "https://github.com/protocolbuffers/protobuf/releases/download/v25.2/protoc-25.2-linux-${PB_ARCH}.zip" -o /tmp/protoc.zip && \
    unzip -o /tmp/protoc.zip -d /usr/local bin/protoc 'include/*' && \
    rm /tmp/protoc.zip && \
    protoc --version

# ---- Install Rust toolchain ----
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH="/usr/local/cargo/bin:${PATH}"

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain 1.92.0 --profile minimal && \
    rustup component add rustfmt clippy

# ---- Install Node.js 20 LTS ----
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get install -y --no-install-recommends nodejs && \
    rm -rf /var/lib/apt/lists/*

# ---- Install tauri-driver ----
RUN cargo install tauri-driver 2>/dev/null

# ---- Install Playwright + Chromium for faucet automation (Phase 3) ----
RUN npx playwright install --with-deps chromium 2>/dev/null

# ---- Set up working directory ----
WORKDIR /app

# ---- Layer: Cargo dependency cache ----
# Copy only manifests first to cache dependency compilation
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src-tauri/Cargo.toml src-tauri/Cargo.toml
COPY src-tauri/build.rs src-tauri/build.rs

# Create stub source files so cargo can resolve the workspace
RUN mkdir -p src && echo "fn main() {}" > src/main.rs && \
    mkdir -p src-tauri/src && echo "fn main() {}" > src-tauri/src/main.rs

# Pre-fetch and compile Rust dependencies (cached unless Cargo.toml/lock changes)
RUN cargo build --manifest-path src-tauri/Cargo.toml 2>/dev/null || true

# Remove stub sources (real sources copied in next stage)
RUN rm -rf src src-tauri/src

# ---- Layer: npm dependency cache ----
COPY package.json package-lock.json* ./
RUN npm ci --ignore-scripts 2>/dev/null

# ---- Layer: Full source ----
COPY . .

# ---- Build the Tauri app in debug mode ----
# Debug mode is faster to compile and sufficient for E2E testing
RUN npm run build 2>/dev/null && \
    cargo build --manifest-path src-tauri/Cargo.toml && \
    test -f src-tauri/target/debug/dash-evo-tool-tauri

# ---- Copy entrypoint and helpers ----
COPY docker/e2e/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# ---- Write .env to app data directory (testnet DAPI addresses) ----
RUN mkdir -p /root/.config/Dash-Evo-Tool && \
    cp /app/.env.example /root/.config/Dash-Evo-Tool/.env 2>/dev/null || true

# Environment for headless display
ENV DISPLAY=:99

ENTRYPOINT ["/entrypoint.sh"]
