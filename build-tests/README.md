# Build Tests for Dash Evo Tool

This directory contains test scripts and documentation for verifying the different Linux build variants of Dash Evo Tool.

## Contents

### Test Scripts

- **`test-linux-builds.sh`** - Comprehensive automated test script that tests all build variants on multiple Linux distributions using Docker
- **`test-single-build.sh`** - Quick script to test a single build on a specific distribution
- **`test-build-locally.sh`** - Build and test all variants locally using Docker (simulates GitHub Actions environment)
- **`test-glibc-compatibility.sh`** - Check GLIBC versions across different Linux distributions

### Documentation

- **`test-linux-builds-manual.md`** - Manual testing guide with Docker commands
- **`quick-test-instructions.md`** - Quick reference for testing the workflow changes

## Quick Start

1. **Check GLIBC versions across distributions:**
   ```bash
   ./test-glibc-compatibility.sh
   ```

2. **Test pre-built releases:**
   ```bash
   # Download releases from GitHub first, then:
   ./test-linux-builds.sh
   ```

3. **Build and test locally:**
   ```bash
   ./test-build-locally.sh
   ```

## Build Variants

The project produces three types of Linux builds:

1. **Standard Build** (`x86_64-linux`)
   - Built on Ubuntu 24.04
   - Requires GLIBC 2.39+
   - Best performance on modern systems

2. **Compatible Build** (`x86_64-linux-compat`)
   - Built on Ubuntu 20.04
   - Requires GLIBC 2.31+
   - Works on most distributions from 2020 onwards

3. **AppImage** (`x86_64-linux-appimage`)
   - Self-contained with all dependencies
   - Works on almost any Linux distribution
   - No installation required