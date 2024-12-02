#!/bin/bash

set -e

VERSION="$1"
PLATFORM="$2"
EXT="$3"

if [ -z "PLATFORM" ]; then
  echo "Error"
  exit 1
fi

create_zip_package() {
  echo "Building ZIP for $PLATFORM version $VERSION"
  echo "extention is:$EXT"

  zip -r $DIST_DIR/dash-evo-tool-"$PLATFORM".zip $BUILD_DIR
}

create_dmg_package() {
  echo "Building DMG for $PLATFORM version $VERSION"
  echo "extention is:$EXT"

  APP_BUNDLE_NAME="$APP_NAME.app"
  APP_BUNDLE_DIR="$BUILD_DIR/$APP_BUNDLE_NAME"
  CONTENTS_DIR="$APP_BUNDLE_DIR/Contents"
  MACOS_DIR="$CONTENTS_DIR/MacOS"
  RESOURCES_DIR="$CONTENTS_DIR/Resources"

  # Create directories for the .app bundle
  mkdir -p "$APP_BUNDLE_DIR"
  mkdir -p "$MACOS_DIR"
  mkdir -p "$RESOURCES_DIR"

  # Copy the binary into the app bundle
  cp "$BUILD_DIR/$APP_NAME" "$MACOS_DIR/"

  ICON_SOURCE="$ROOT_PATH/mac_os/AppIcons/appstore.png"
  ICONSET_DIR="$BUILD_DIR/AppIcon.iconset"
  mkdir -p "$ICONSET_DIR"

  sips -z 16 16     "$ICON_SOURCE" --out "$ICONSET_DIR/icon_16x16.png"
  sips -z 32 32     "$ICON_SOURCE" --out "$ICONSET_DIR/icon_16x16@2x.png"
  sips -z 32 32     "$ICON_SOURCE" --out "$ICONSET_DIR/icon_32x32.png"
  sips -z 64 64     "$ICON_SOURCE" --out "$ICONSET_DIR/icon_32x32@2x.png"
  sips -z 128 128   "$ICON_SOURCE" --out "$ICONSET_DIR/icon_128x128.png"
  sips -z 256 256   "$ICON_SOURCE" --out "$ICONSET_DIR/icon_128x128@2x.png"
  sips -z 256 256   "$ICON_SOURCE" --out "$ICONSET_DIR/icon_256x256.png"
  sips -z 512 512   "$ICON_SOURCE" --out "$ICONSET_DIR/icon_256x256@2x.png"
  sips -z 512 512   "$ICON_SOURCE" --out "$ICONSET_DIR/icon_512x512.png"
  cp "$ICON_SOURCE" "$ICONSET_DIR/icon_512x512@2x.png"

  # Convert iconset to .icns
  iconutil -c icns "$ICONSET_DIR" -o "$RESOURCES_DIR/AppIcon.icns"

  # Clean up the iconset directory
  rm -rf "$ICONSET_DIR"
# Create a minimal Info.plist file
  cat <<EOF > "$CONTENTS_DIR/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
"http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>$APP_NAME</string>
  <key>CFBundleDisplayName</key>
  <string>$APP_NAME</string>
  <key>CFBundleExecutable</key>
  <string>$APP_NAME</string>
  <key>CFBundleIdentifier</key>
  <string>com.example.$APP_NAME</string>
  <key>CFBundleVersion</key>
  <string>$VERSION</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleSignature</key>
  <string>????</string>
  <key>LSMinimumSystemVersion</key>
  <string>10.9</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon.icns</string>
</dict>
</plist>
EOF

  # Create the .dmg directory structure
  DMG_DIR="$BUILD_DIR/dmg_content"
  mkdir -p "$DMG_DIR"

  # Copy the .app bundle into the dmg content directory
  cp -R "$APP_BUNDLE_DIR" "$DMG_DIR/"

  # Create a symbolic link to the Applications folder
  ln -s /Applications "$DMG_DIR/Applications"

  # Create the .dmg file
  hdiutil create -volname "$APP_NAME Installer" \
                 -srcfolder "$DMG_DIR" \
                 -ov -format UDZO \
                 "$DIST_DIR/$APP_NAME-$PLATFORM.dmg"
}

echo "Starting"

FULL_PATH=$(realpath "$0")
DIR_PATH=$(dirname "$FULL_PATH")
ROOT_PATH=$(dirname "$DIR_PATH")

APP_NAME="dash-evo-tool"
BUILD_DIR="$ROOT_PATH/dash-evo-tool"
DIST_DIR="$ROOT_PATH/dist"

mkdir -p "$DIST_DIR"

case "$PLATFORM" in
  x86_64-mac)
    create_dmg_package
    ;;
  arm64-mac)
    create_dmg_package
    ;;
  x86_64-linux)
    create_zip_package
    ;;
  arm64-linux)
    create_zip_package
    ;;
  windows)
    create_zip_package
    ;;
  *)
    echo "Invalid command."
    echo "$cmd_usage"
    exit 1
    ;;
esac

rm -rf "$BUILD_DIR"
echo "Done."