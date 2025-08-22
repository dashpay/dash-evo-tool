#!/bin/bash

# Simple script to test GLIBC compatibility without building
# This helps understand what GLIBC versions different distributions have

echo "=== GLIBC Version Check on Different Linux Distributions ==="
echo ""

# Function to check GLIBC version in a container
check_glibc() {
    local distro=$1
    local image=$2
    
    echo -n "$distro: "
    docker run --rm "$image" bash -c "ldd --version 2>/dev/null | head -n 1 | grep -oE '[0-9]+\.[0-9]+' | head -1" 2>/dev/null || echo "Failed to check"
}

echo "Checking GLIBC versions..."
echo "=========================="

# Ubuntu versions
check_glibc "Ubuntu 18.04 LTS" "ubuntu:18.04"
check_glibc "Ubuntu 20.04 LTS" "ubuntu:20.04"
check_glibc "Ubuntu 22.04 LTS" "ubuntu:22.04"
check_glibc "Ubuntu 24.04 LTS" "ubuntu:24.04"

echo ""

# Debian versions
check_glibc "Debian 10 (Buster)" "debian:10"
check_glibc "Debian 11 (Bullseye)" "debian:11"
check_glibc "Debian 12 (Bookworm)" "debian:12"

echo ""

# Other distributions
check_glibc "CentOS Stream 9" "quay.io/centos/centos:stream9"
check_glibc "Fedora 38" "fedora:38"
check_glibc "Fedora 39" "fedora:39"
check_glibc "Alpine Linux" "alpine:latest"

echo ""
echo "Summary:"
echo "========"
echo "- GLIBC 2.31 (Ubuntu 20.04 compatible build) works on:"
echo "  Ubuntu 20.04+, Debian 11+, CentOS Stream 9+, Fedora 34+"
echo ""
echo "- GLIBC 2.39 (Ubuntu 24.04 standard build) works on:"
echo "  Ubuntu 24.04+, Fedora 39+, and very recent distributions"
echo ""
echo "- Alpine Linux uses musl libc instead of GLIBC (requires different build)"