# Testing Linux Builds for GLIBC Compatibility

This guide helps you test the different Linux builds to ensure they work correctly across various distributions.

## Quick Test with Docker

### 1. First, trigger a test build

You'll need to push your changes and create a test tag:

```bash
git add .github/workflows/release.yml README.md
git commit -m "feat: add multiple Linux build variants for GLIBC compatibility"
git push origin v0.9-dev

# Create a test tag to trigger the workflow
git tag v0.9-dev.test1
git push origin v0.9-dev.test1
```

### 2. Download the artifacts

Once the GitHub Actions workflow completes, download the Linux builds from the release page.

### 3. Test with Docker

Test each build variant on different distributions:

#### Test Standard Build (should fail on older systems)
```bash
# Should work - Ubuntu 24.04 has GLIBC 2.39
docker run --rm -it -v "$PWD:/test" ubuntu:24.04 bash -c "
  cd /test && apt-get update && apt-get install -y unzip
  unzip -q dash-evo-tool-x86_64-linux.zip
  cd dash-evo-tool && chmod +x dash-evo-tool
  ldd --version | head -n 1
  ./dash-evo-tool --version
"

# Should fail - Ubuntu 20.04 has GLIBC 2.31
docker run --rm -it -v "$PWD:/test" ubuntu:20.04 bash -c "
  cd /test && apt-get update && apt-get install -y unzip
  unzip -q dash-evo-tool-x86_64-linux.zip
  cd dash-evo-tool && chmod +x dash-evo-tool
  ldd --version | head -n 1
  ./dash-evo-tool --version || echo 'Expected failure: GLIBC too old'
"
```

#### Test Compatible Build (should work on older systems)
```bash
# Should work on Ubuntu 20.04 and newer
docker run --rm -it -v "$PWD:/test" ubuntu:20.04 bash -c "
  cd /test && apt-get update && apt-get install -y unzip
  unzip -q dash-evo-tool-x86_64-linux-compat.zip
  cd dash-evo-tool && chmod +x dash-evo-tool
  ldd --version | head -n 1
  ./dash-evo-tool --version
"

# Should also work on Debian 11
docker run --rm -it -v "$PWD:/test" debian:11 bash -c "
  cd /test && apt-get update && apt-get install -y unzip
  unzip -q dash-evo-tool-x86_64-linux-compat.zip
  cd dash-evo-tool && chmod +x dash-evo-tool
  ldd --version | head -n 1
  ./dash-evo-tool --version
"
```

#### Test AppImage (should work almost everywhere)
```bash
# Test on various distributions
for distro in ubuntu:20.04 ubuntu:18.04 debian:11 fedora:38; do
  echo "Testing AppImage on $distro..."
  docker run --rm -it -v "$PWD:/test" $distro bash -c "
    cd /test
    # Install unzip (commands vary by distro)
    if command -v apt-get &> /dev/null; then
      apt-get update && apt-get install -y unzip
    elif command -v dnf &> /dev/null; then
      dnf install -y unzip
    fi
    unzip -q dash-evo-tool-x86_64-linux-appimage.zip
    chmod +x dash-evo-tool.AppImage
    # AppImage might need --appimage-extract-and-run in containers
    ./dash-evo-tool.AppImage --appimage-extract-and-run --version || ./dash-evo-tool.AppImage --version
  "
done
```

## Expected Results

1. **Standard Build** (`x86_64-linux`):
   - ✅ Works on Ubuntu 24.04, Fedora 39+, and other very recent distributions
   - ❌ Fails on Ubuntu 22.04 and older with GLIBC version errors

2. **Compatible Build** (`x86_64-linux-compat`):
   - ✅ Works on Ubuntu 20.04, Debian 11, CentOS Stream 9, and most 2020+ distributions
   - ✅ Also works on newer systems
   - ❌ May fail on very old systems (Ubuntu 18.04 and older)

3. **AppImage** (`x86_64-linux-appimage`):
   - ✅ Works on almost all Linux distributions
   - ✅ Self-contained with all dependencies
   - ⚠️ May need `--appimage-extract-and-run` flag in Docker containers without FUSE

## Checking GLIBC Dependencies

To see what GLIBC version a binary requires:
```bash
# Check required GLIBC versions
objdump -T dash-evo-tool | grep GLIBC | sed 's/.*GLIBC_\([.0-9]*\).*/\1/g' | sort -V | tail -1

# Or use ldd to see all dependencies
ldd dash-evo-tool
```

## Common GLIBC Versions

- Ubuntu 18.04: GLIBC 2.27
- Ubuntu 20.04: GLIBC 2.31
- Ubuntu 22.04: GLIBC 2.35
- Ubuntu 24.04: GLIBC 2.39
- Debian 11: GLIBC 2.31
- Debian 12: GLIBC 2.36
- RHEL/CentOS 9: GLIBC 2.34