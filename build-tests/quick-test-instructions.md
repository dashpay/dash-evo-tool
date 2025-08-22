# Quick Test Instructions for Linux Build Variants

Based on the GLIBC version check, here's what we've confirmed:

## GLIBC Versions by Distribution:
- Ubuntu 18.04: GLIBC 2.27
- Ubuntu 20.04: GLIBC 2.31 ✓ (Compatible build target)
- Ubuntu 22.04: GLIBC 2.35
- Ubuntu 24.04: GLIBC 2.39 ✓ (Standard build target)
- Debian 11: GLIBC 2.31
- Debian 12: GLIBC 2.36

## What the changes will do:

1. **Standard Build** (built on Ubuntu 24.04):
   - Will require GLIBC 2.39
   - Only works on Ubuntu 24.04, Fedora 39+, and very recent distros
   - This matches your current situation where users complain about GLIBC 2.39

2. **Compatible Build** (built on Ubuntu 20.04):
   - Will require GLIBC 2.31
   - Works on Ubuntu 20.04, Debian 11, CentOS Stream 9, and most 2020+ distros
   - This solves the compatibility issue for most users

3. **AppImage**:
   - Self-contained with all dependencies
   - Works on almost any Linux distribution
   - Best fallback option

## To test the changes:

1. **Commit and push the changes:**
```bash
git add .github/workflows/release.yml README.md
git commit -m "feat: add multiple Linux build variants for GLIBC compatibility"
git push origin v0.9-dev
```

2. **Create a test release:**
```bash
# Create a test tag to trigger the workflow
git tag v0.9-dev.test1
git push origin v0.9-dev.test1
```

3. **Monitor the build:**
- Go to https://github.com/dashpay/dash-evo-tool/actions
- Watch the "Release Dash Evo Tool" workflow
- It should create 5 Linux packages instead of 2

4. **Download and test:**
Once complete, download the artifacts and test:

```bash
# Test compatible build on Ubuntu 20.04
docker run --rm -it -v "$PWD:/test" ubuntu:20.04 bash -c "
  apt-get update && apt-get install -y unzip
  cd /test
  unzip -q dash-evo-tool-x86_64-linux-compat.zip
  cd dash-evo-tool
  chmod +x dash-evo-tool
  ./dash-evo-tool --version
"
```

## Expected outcomes:

✅ **Compatible build** should work on Ubuntu 20.04 (GLIBC 2.31)
❌ **Standard build** should fail on Ubuntu 20.04 with GLIBC error
✅ **AppImage** should work everywhere (might need --appimage-extract-and-run in containers)

This approach gives users three options:
- Modern systems: Use standard build for best performance
- Older systems: Use compatible build
- Any issues: Use AppImage as universal fallback