#!/bin/bash

# Test script for Dash Evo Tool Linux builds
# This script tests different Linux distributions to verify GLIBC compatibility

set -e

echo "=== Dash Evo Tool Linux Build Testing ==="
echo "This script will test the different Linux builds using Docker containers"
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to test a build on a specific distribution
test_build() {
    local distro=$1
    local image=$2
    local build_file=$3
    local expected_result=$4
    
    echo -e "${YELLOW}Testing $distro with $build_file...${NC}"
    
    # Create a test script that will run inside the container
    cat > test_in_container.sh << 'EOF'
#!/bin/bash
cd /test
unzip -q *.zip
cd dash-evo-tool
chmod +x dash-evo-tool

# Check GLIBC version
echo "GLIBC version in container:"
ldd --version | head -n 1

# Try to run the binary
echo "Testing binary..."
if timeout 5s ./dash-evo-tool --version 2>&1; then
    echo "SUCCESS: Binary runs!"
    exit 0
else
    echo "Checking dependencies..."
    ldd ./dash-evo-tool || true
    exit 1
fi
EOF
    
    chmod +x test_in_container.sh
    
    # Run the test in Docker
    if docker run --rm \
        -v "$PWD/test_in_container.sh:/test_in_container.sh" \
        -v "$PWD/$build_file:/test/$build_file" \
        "$image" \
        /test_in_container.sh; then
        
        if [ "$expected_result" = "pass" ]; then
            echo -e "${GREEN}✓ PASS: $distro works with $build_file as expected${NC}"
        else
            echo -e "${RED}✗ UNEXPECTED: $distro works with $build_file but was expected to fail${NC}"
        fi
    else
        if [ "$expected_result" = "fail" ]; then
            echo -e "${GREEN}✓ EXPECTED: $distro fails with $build_file as expected${NC}"
        else
            echo -e "${RED}✗ FAIL: $distro doesn't work with $build_file${NC}"
        fi
    fi
    
    echo ""
    rm -f test_in_container.sh
}

# Test AppImage
test_appimage() {
    local distro=$1
    local image=$2
    
    echo -e "${YELLOW}Testing AppImage on $distro...${NC}"
    
    cat > test_appimage.sh << 'EOF'
#!/bin/bash
cd /test
unzip -q *.zip
chmod +x dash-evo-tool.AppImage

# Check GLIBC version
echo "GLIBC version in container:"
ldd --version | head -n 1

# AppImages need FUSE or can be extracted
echo "Testing AppImage..."
if timeout 5s ./dash-evo-tool.AppImage --version 2>&1; then
    echo "SUCCESS: AppImage runs!"
    exit 0
else
    # Try extracting if FUSE is not available
    echo "FUSE might not be available, trying --appimage-extract-and-run..."
    if timeout 5s ./dash-evo-tool.AppImage --appimage-extract-and-run --version 2>&1; then
        echo "SUCCESS: AppImage runs with extraction!"
        exit 0
    else
        echo "FAIL: AppImage doesn't work"
        exit 1
    fi
fi
EOF
    
    chmod +x test_appimage.sh
    
    if docker run --rm \
        -v "$PWD/test_appimage.sh:/test_appimage.sh" \
        -v "$PWD/$3:/test/$3" \
        --cap-add SYS_ADMIN \
        --device /dev/fuse \
        "$image" \
        /test_appimage.sh; then
        echo -e "${GREEN}✓ PASS: AppImage works on $distro${NC}"
    else
        echo -e "${RED}✗ FAIL: AppImage doesn't work on $distro${NC}"
    fi
    
    echo ""
    rm -f test_appimage.sh
}

# Check if we have the release files
echo "Checking for release files..."
echo "Please ensure you have built the releases or downloaded them to this directory:"
echo "  - dash-evo-tool-x86_64-linux.zip (standard build)"
echo "  - dash-evo-tool-x86_64-linux-compat.zip (compatible build)"
echo "  - dash-evo-tool-x86_64-linux-appimage.zip (AppImage)"
echo ""
echo "Press Enter to continue or Ctrl+C to cancel..."
read

# Test matrix
echo -e "${YELLOW}=== Testing Standard Build (GLIBC 2.39) ===${NC}"
test_build "Ubuntu 24.04" "ubuntu:24.04" "dash-evo-tool-x86_64-linux.zip" "pass"
test_build "Ubuntu 22.04" "ubuntu:22.04" "dash-evo-tool-x86_64-linux.zip" "fail"
test_build "Ubuntu 20.04" "ubuntu:20.04" "dash-evo-tool-x86_64-linux.zip" "fail"
test_build "Debian 12" "debian:12" "dash-evo-tool-x86_64-linux.zip" "fail"
test_build "CentOS Stream 9" "quay.io/centos/centos:stream9" "dash-evo-tool-x86_64-linux.zip" "fail"

echo -e "${YELLOW}=== Testing Compatible Build (GLIBC 2.31) ===${NC}"
test_build "Ubuntu 24.04" "ubuntu:24.04" "dash-evo-tool-x86_64-linux-compat.zip" "pass"
test_build "Ubuntu 22.04" "ubuntu:22.04" "dash-evo-tool-x86_64-linux-compat.zip" "pass"
test_build "Ubuntu 20.04" "ubuntu:20.04" "dash-evo-tool-x86_64-linux-compat.zip" "pass"
test_build "Debian 11" "debian:11" "dash-evo-tool-x86_64-linux-compat.zip" "pass"
test_build "Debian 12" "debian:12" "dash-evo-tool-x86_64-linux-compat.zip" "pass"
test_build "CentOS Stream 9" "quay.io/centos/centos:stream9" "dash-evo-tool-x86_64-linux-compat.zip" "pass"
test_build "Ubuntu 18.04" "ubuntu:18.04" "dash-evo-tool-x86_64-linux-compat.zip" "fail"

echo -e "${YELLOW}=== Testing AppImage ===${NC}"
test_appimage "Ubuntu 24.04" "ubuntu:24.04" "dash-evo-tool-x86_64-linux-appimage.zip"
test_appimage "Ubuntu 20.04" "ubuntu:20.04" "dash-evo-tool-x86_64-linux-appimage.zip"
test_appimage "Ubuntu 18.04" "ubuntu:18.04" "dash-evo-tool-x86_64-linux-appimage.zip"
test_appimage "Debian 11" "debian:11" "dash-evo-tool-x86_64-linux-appimage.zip"
test_appimage "CentOS Stream 9" "quay.io/centos/centos:stream9" "dash-evo-tool-x86_64-linux-appimage.zip"

echo -e "${GREEN}=== Testing Complete ===${NC}"
echo "Summary:"
echo "- Standard build should work on Ubuntu 24.04+ (GLIBC 2.39+)"
echo "- Compatible build should work on Ubuntu 20.04+ and most 2020+ distributions (GLIBC 2.31+)"
echo "- AppImage should work on most Linux distributions"