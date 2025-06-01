#!/bin/bash

# Quick test script to verify a single build manually
# Usage: ./test-single-build.sh <distro> <build-file>
# Example: ./test-single-build.sh ubuntu:20.04 dash-evo-tool-x86_64-linux-compat.zip

if [ $# -lt 2 ]; then
    echo "Usage: $0 <docker-image> <build-file>"
    echo "Example: $0 ubuntu:20.04 dash-evo-tool-x86_64-linux-compat.zip"
    exit 1
fi

DISTRO=$1
BUILD_FILE=$2

echo "Testing $BUILD_FILE on $DISTRO..."

docker run --rm -it \
    -v "$PWD/$BUILD_FILE:/test/$BUILD_FILE" \
    "$DISTRO" \
    /bin/bash -c "
        cd /test
        apt-get update && apt-get install -y unzip ldd || yum install -y unzip
        unzip -q $BUILD_FILE
        cd dash-evo-tool
        chmod +x dash-evo-tool
        echo '=== GLIBC Version ==='
        ldd --version | head -n 1
        echo '=== Binary Dependencies ==='
        ldd ./dash-evo-tool
        echo '=== Testing Binary ==='
        ./dash-evo-tool --version || echo 'Failed to run'
    "