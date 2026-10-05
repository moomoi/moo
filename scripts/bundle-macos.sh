#!/usr/bin/env bash
# Assemble dist/Moo.app: the native shell binary plus built plugins in Contents/Resources/plugins,
# and the icons from packaging/ (see scripts/make-icons.sh).
# LSUIElement keeps it out of the Dock and app switcher. Signs ad hoc unless SIGN_IDENTITY is set.
#   SKIP_BUILD=1   reuse what is already built
#   MOO_BIN, MOO_PLUGIN_DIST   what to bundle (default app/dist/moo, plugins/dist; release builds
#                  pass dist/universal/moo and dist/universal/plugins from build-universal.sh)
#   SIGN_IDENTITY  e.g. "Developer ID Application: ..." (adds hardened runtime + entitlements)
#   VERSION        CFBundleShortVersionString, e.g. 1.2.0
#   BUILD          CFBundleVersion; must grow with every release (CI uses the run number)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${VERSION:-0.1.0}"
BUILD="${BUILD:-$VERSION}"
BUNDLE_ID="moi.moo.launcher"
APP="$ROOT/dist/Moo.app"
BIN="${MOO_BIN:-$ROOT/app/dist/moo}"
PLUGIN_DIST="${MOO_PLUGIN_DIST:-$ROOT/plugins/dist}"

if [ -z "${SKIP_BUILD:-}" ]; then
  bash "$ROOT/plugins/build.sh"
  bash "$ROOT/app/build.sh"
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/plugins"
cp "$BIN" "$APP/Contents/MacOS/moo"
for p in "$PLUGIN_DIST"/*.lib "$PLUGIN_DIST"/*.tishc; do
  [ -e "$p" ] && cp "$p" "$APP/Contents/Resources/plugins/"
done
cp "$ROOT/packaging/AppIcon.icns" "$ROOT/packaging/MenuBarIcon.tiff" "$ROOT/packaging/SearchIcon.png" "$APP/Contents/Resources/"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Moo</string>
  <key>CFBundleDisplayName</key><string>Moo</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>moo</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$BUILD</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSContactsUsageDescription</key><string>Moo searches your contacts when you type a name.</string>
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
"${SIGN[@]}" --identifier "$BUNDLE_ID" --entitlements "$ROOT/packaging/entitlements.plist" "$APP"
codesign --verify --strict "$APP"

du -sh "$APP"
echo "built $APP"
