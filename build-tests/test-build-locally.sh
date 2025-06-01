#!/bin/bash

# Script to build and test Dash Evo Tool locally using Docker
# This simulates the GitHub Actions build environment

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== Building Dash Evo Tool Linux Variants Locally ==="

# Create build directory
mkdir -p build-test
cd build-test

# Function to build in a container
build_in_container() {
    local name=$1
    local image=$2
    local target=$3
    
    echo -e "${YELLOW}Building $name using $image...${NC}"
    
    # Create Dockerfile for building
    cat > Dockerfile << EOF
FROM $image

# Install dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    clang \
    cmake \
    unzip \
    libsqlite3-dev \
    zip \
    curl \
    git \
    libssl-dev

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:\${PATH}"

# Install protoc
RUN curl -OL https://github.com/protocolbuffers/protobuf/releases/download/v25.2/protoc-25.2-linux-x86_64.zip && \
    unzip -o protoc-25.2-linux-x86_64.zip -d /usr/local bin/protoc && \
    unzip -o protoc-25.2-linux-x86_64.zip -d /usr/local 'include/*' && \
    rm -f protoc-25.2-linux-x86_64.zip

WORKDIR /build
COPY . .

# Add target
RUN rustup target add $target

# Build
RUN cargo build --release --target $target

# Package
RUN mkdir -p dash-evo-tool && \
    cp target/$target/release/dash-evo-tool dash-evo-tool/ && \
    cp .env.example dash-evo-tool/.env && \
    cp -r dash_core_configs dash-evo-tool/ && \
    zip -r dash-evo-tool-$name.zip dash-evo-tool/
EOF

    # Copy source code
    echo "Copying source code..."
    cp -r ../* . 2>/dev/null || true
    
    # Build in container
    docker build -t dash-evo-tool-build-$name .
    
    # Extract the built artifact
    docker run --rm -v "$PWD:/output" dash-evo-tool-build-$name \
        cp /build/dash-evo-tool-$name.zip /output/
    
    echo -e "${GREEN}✓ Built $name${NC}"
    echo ""
}

# Build standard version (Ubuntu 24.04)
build_in_container "x86_64-linux" "ubuntu:24.04" "x86_64-unknown-linux-gnu"

# Build compatible version (Ubuntu 20.04)
build_in_container "x86_64-linux-compat" "ubuntu:20.04" "x86_64-unknown-linux-gnu"

# Build AppImage
echo -e "${YELLOW}Building AppImage...${NC}"
cat > Dockerfile.appimage << 'EOF'
FROM ubuntu:20.04

ENV DEBIAN_FRONTEND=noninteractive

# Install dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    clang \
    cmake \
    unzip \
    libsqlite3-dev \
    zip \
    curl \
    git \
    libssl-dev \
    wget \
    libxcb-xfixes0-dev \
    libxcb-shape0-dev \
    libxcb-randr0-dev \
    libxcb-xkb-dev \
    libxkbcommon-x11-dev \
    file

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Install protoc
RUN curl -OL https://github.com/protocolbuffers/protobuf/releases/download/v25.2/protoc-25.2-linux-x86_64.zip && \
    unzip -o protoc-25.2-linux-x86_64.zip -d /usr/local bin/protoc && \
    unzip -o protoc-25.2-linux-x86_64.zip -d /usr/local 'include/*' && \
    rm -f protoc-25.2-linux-x86_64.zip

# Install linuxdeploy
RUN wget -q https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage && \
    chmod +x linuxdeploy-x86_64.AppImage && \
    ./linuxdeploy-x86_64.AppImage --appimage-extract && \
    mv squashfs-root/usr/bin/linuxdeploy /usr/local/bin/ && \
    rm -rf squashfs-root linuxdeploy-x86_64.AppImage

WORKDIR /build
COPY . .

# Build
RUN cargo build --release --target x86_64-unknown-linux-gnu

# Create AppImage structure
RUN mkdir -p dash-evo-tool && \
    cp target/x86_64-unknown-linux-gnu/release/dash-evo-tool dash-evo-tool/ && \
    cp .env.example dash-evo-tool/.env && \
    cp -r dash_core_configs dash-evo-tool/

# Create AppDir
RUN mkdir -p AppDir/usr/bin && \
    mkdir -p AppDir/usr/share/applications && \
    mkdir -p AppDir/usr/share/icons/hicolor/256x256/apps && \
    cp dash-evo-tool/dash-evo-tool AppDir/usr/bin/ && \
    cp -r dash-evo-tool/dash_core_configs AppDir/usr/bin/ && \
    cp dash-evo-tool/.env AppDir/usr/bin/

# Create desktop file
RUN echo '[Desktop Entry]' > AppDir/usr/share/applications/dash-evo-tool.desktop && \
    echo 'Name=Dash Evo Tool' >> AppDir/usr/share/applications/dash-evo-tool.desktop && \
    echo 'Exec=dash-evo-tool' >> AppDir/usr/share/applications/dash-evo-tool.desktop && \
    echo 'Icon=dash-evo-tool' >> AppDir/usr/share/applications/dash-evo-tool.desktop && \
    echo 'Type=Application' >> AppDir/usr/share/applications/dash-evo-tool.desktop && \
    echo 'Categories=Utility;' >> AppDir/usr/share/applications/dash-evo-tool.desktop

# Copy icon if exists, otherwise create placeholder
RUN if [ -f mac_os/AppIcons/Assets.xcassets/AppIcon.appiconset/256.png ]; then \
        cp mac_os/AppIcons/Assets.xcassets/AppIcon.appiconset/256.png AppDir/usr/share/icons/hicolor/256x256/apps/dash-evo-tool.png; \
    else \
        echo "No icon found, using placeholder"; \
        touch AppDir/usr/share/icons/hicolor/256x256/apps/dash-evo-tool.png; \
    fi

# Create AppRun script
RUN echo '#!/bin/bash' > AppDir/AppRun && \
    echo 'SELF=$(readlink -f "$0")' >> AppDir/AppRun && \
    echo 'HERE=${SELF%/*}' >> AppDir/AppRun && \
    echo 'export PATH="${HERE}/usr/bin:${PATH}"' >> AppDir/AppRun && \
    echo 'export LD_LIBRARY_PATH="${HERE}/usr/lib:${LD_LIBRARY_PATH}"' >> AppDir/AppRun && \
    echo 'cd "${HERE}/usr/bin"' >> AppDir/AppRun && \
    echo 'exec "${HERE}/usr/bin/dash-evo-tool" "$@"' >> AppDir/AppRun && \
    chmod +x AppDir/AppRun

# Create AppImage
RUN linuxdeploy --appdir AppDir --output appimage && \
    mv Dash_Evo_Tool*.AppImage dash-evo-tool.AppImage && \
    zip dash-evo-tool-x86_64-linux-appimage.zip dash-evo-tool.AppImage
EOF

docker build -f Dockerfile.appimage -t dash-evo-tool-build-appimage .
docker run --rm -v "$PWD:/output" dash-evo-tool-build-appimage \
    cp /build/dash-evo-tool-x86_64-linux-appimage.zip /output/

echo -e "${GREEN}✓ Built AppImage${NC}"
echo ""

# Now test the builds
echo -e "${YELLOW}=== Testing Builds ===${NC}"

# Test function
test_build() {
    local distro=$1
    local image=$2
    local build_file=$3
    
    echo -e "${YELLOW}Testing $build_file on $distro...${NC}"
    
    docker run --rm \
        -v "$PWD/$build_file:/test.zip" \
        "$image" \
        bash -c "
            apt-get update && apt-get install -y unzip > /dev/null 2>&1
            cd /tmp
            unzip -q /test.zip
            cd dash-evo-tool
            chmod +x dash-evo-tool
            echo 'GLIBC version:'
            ldd --version | head -n 1
            echo 'Testing binary...'
            if ./dash-evo-tool --version 2>&1; then
                echo -e '\033[0;32m✓ SUCCESS: Binary runs!\033[0m'
            else
                echo -e '\033[0;31m✗ FAIL: Binary does not run\033[0m'
                echo 'Dependencies:'
                ldd ./dash-evo-tool 2>&1 | head -20
            fi
        "
    echo ""
}

# Test AppImage
test_appimage() {
    local distro=$1
    local image=$2
    
    echo -e "${YELLOW}Testing AppImage on $distro...${NC}"
    
    docker run --rm \
        -v "$PWD/dash-evo-tool-x86_64-linux-appimage.zip:/test.zip" \
        "$image" \
        bash -c "
            if command -v apt-get &> /dev/null; then
                apt-get update && apt-get install -y unzip file > /dev/null 2>&1
            fi
            cd /tmp
            unzip -q /test.zip
            chmod +x dash-evo-tool.AppImage
            echo 'Testing AppImage...'
            if ./dash-evo-tool.AppImage --appimage-extract-and-run --version 2>&1; then
                echo -e '\033[0;32m✓ SUCCESS: AppImage runs!\033[0m'
            else
                echo -e '\033[0;31m✗ FAIL: AppImage does not run\033[0m'
            fi
        "
    echo ""
}

# Test builds
echo -e "${YELLOW}Testing standard build (should only work on very recent systems)${NC}"
test_build "Ubuntu 24.04" "ubuntu:24.04" "dash-evo-tool-x86_64-linux.zip"
test_build "Ubuntu 20.04" "ubuntu:20.04" "dash-evo-tool-x86_64-linux.zip"

echo -e "${YELLOW}Testing compatible build (should work on older systems)${NC}"
test_build "Ubuntu 20.04" "ubuntu:20.04" "dash-evo-tool-x86_64-linux-compat.zip"
test_build "Debian 11" "debian:11" "dash-evo-tool-x86_64-linux-compat.zip"

echo -e "${YELLOW}Testing AppImage${NC}"
test_appimage "Ubuntu 20.04" "ubuntu:20.04"
test_appimage "Ubuntu 18.04" "ubuntu:18.04"

echo -e "${GREEN}=== Build and Test Complete ===${NC}"
echo "Built packages:"
ls -la *.zip

cd ..
echo ""
echo "Build artifacts are in the build-test/ directory"