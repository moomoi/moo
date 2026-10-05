#!/usr/bin/env bash
# Check a release DMG the way a user's Mac will see it: mount it, verify the signature and
# entitlements, check every Mach-O has both slices, and run `moo --version` on each architecture.
#
#   bash scripts/verify-release.sh dist/release/Moo-macos-universal.dmg
#
# With REQUIRE_NOTARIZED=1 (CI on main) a missing Developer ID signature or ticket is an error.
set -euo pipefail
DMG="${1:?usage: verify-release.sh <file.dmg>}"
MNT="$(mktemp -d)"
hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG" >/dev/null
trap 'hdiutil detach -quiet "$MNT" || true; rmdir "$MNT" 2>/dev/null || true' EXIT
APP="$MNT/Moo.app"
fail() { echo "error: $*" >&2; exit 1; }

[ -d "$APP" ] || fail "no Moo.app in $DMG"
[ -L "$MNT/Applications" ] || fail "no Applications link in $DMG"
codesign --verify --deep --strict --verbose=2 "$APP"
for lib in "$APP"/Contents/Resources/plugins/*.lib; do
  [ -e "$lib" ] && codesign --verify --strict "$lib"
done
case "$(codesign -d --entitlements - "$APP" 2>/dev/null)" in
  *personal-information.addressbook*) ;;
  *) fail "Moo.app is missing the Contacts entitlement" ;;
esac

for f in "$APP/Contents/MacOS/moo" "$APP"/Contents/Resources/plugins/*.lib; do
  [ -e "$f" ] || continue
  archs="$(lipo -archs "$f")"
  echo "$(basename "$f"): $archs"
  case "$archs" in
    *x86_64*arm64* | *arm64*x86_64*) ;;
    *) fail "$(basename "$f") is not universal ($archs)" ;;
  esac
done

echo "arm64: $(arch -arm64 "$APP/Contents/MacOS/moo" --version)"
if arch -x86_64 /usr/bin/true 2>/dev/null; then
  echo "x86_64: $(arch -x86_64 "$APP/Contents/MacOS/moo" --version)"
else
  echo "::warning::Rosetta is not installed; skipped running the x86_64 slice"
fi

AUTHORITY="$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=//p' | head -1)"
echo "signed by: ${AUTHORITY:-ad hoc}"
if [ -n "${REQUIRE_NOTARIZED:-}" ]; then
  case "$AUTHORITY" in
    "Developer ID Application:"*) ;;
    *) fail "Moo.app is not signed with a Developer ID certificate" ;;
  esac
  xcrun stapler validate "$APP"
  xcrun stapler validate "$DMG"
  spctl -a -t exec -vv "$APP"
  spctl -a -t open --context context:primary-signature -vv "$DMG"
fi
echo "verified $DMG"
