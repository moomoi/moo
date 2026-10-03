#!/usr/bin/env bash
# Assemble dist/Nimble.app: the native shell binary plus built plugins in Contents/Resources/plugins.
# LSUIElement keeps it out of the Dock and app switcher. Signs ad hoc unless SIGN_IDENTITY is set.
#   SKIP_BUILD=1   reuse app/dist/nimble and plugins/dist
#   SIGN_IDENTITY  e.g. "Developer ID Application: ..." (adds hardened runtime)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${VERSION:-0.1.0}"
APP="$ROOT/dist/Nimble.app"

if [ -z "${SKIP_BUILD:-}" ]; then
  bash "$ROOT/plugins/build.sh"
  bash "$ROOT/app/build.sh"
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/plugins"
cp "$ROOT/app/dist/nimble" "$APP/Contents/MacOS/nimble"
for p in "$ROOT"/plugins/dist/*.lib "$ROOT"/plugins/dist/*.tishc; do
  [ -e "$p" ] && cp "$p" "$APP/Contents/Resources/plugins/"
done

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Nimble</string>
  <key>CFBundleDisplayName</key><string>Nimble</string>
  <key>CFBundleIdentifier</key><string>dev.nimble.launcher</string>
  <key>CFBundleExecutable</key><string>nimble</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Plugins are dlopen'd at runtime, so each is signed on its own; --deep does not reach Resources.
if [ -n "${SIGN_IDENTITY:-}" ]; then
  SIGN=(codesign --force --timestamp --options runtime -s "$SIGN_IDENTITY")
else
  SIGN=(codesign --force -s -)
fi
for lib in "$APP"/Contents/Resources/plugins/*.lib; do
  [ -e "$lib" ] && "${SIGN[@]}" "$lib"
done
"${SIGN[@]}" "$APP"
codesign --verify --strict "$APP"

du -sh "$APP"
echo "built $APP"
