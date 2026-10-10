#!/usr/bin/env bash
# Make every Moo icon from the one square source, packaging/icon.png (1024x1024, full bleed):
#
#   packaging/AppIcon.icns       app icon: the macOS rounded square with the standard margin
#   packaging/AppIcon.ico        the Windows app and tray icon: 16 to 256 px
#   packaging/windows/*.png      MSIX logos: StoreLogo 50, Square44x44Logo, Square150x150Logo
#   packaging/MenuBarIcon.tiff   menu bar template: the snout as a black line icon on clear, 1x and 2x
#   packaging/SearchIcon.png     the same line snout, a little heavier, for the search field
#   web/public/icon.png, favicon.png   the site's logo and favicon
#
#   bash scripts/make-icons.sh
#
# The outputs are committed, so builds do not need ImageMagick. Run this again after changing
# icon.png. Needs ImageMagick 7 (`magick`), plus iconutil and tiffutil from macOS.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/packaging/icon.png"
command -v magick >/dev/null || { echo "error: needs ImageMagick 7 (brew install imagemagick)" >&2; exit 1; }
[ -f "$SRC" ] || { echo "error: no $SRC" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# App icon. Apple's grid: the artwork is an 824pt rounded square centred on a 1024pt canvas, with
# a soft shadow below it.
magick "$SRC" -resize 824x824 \
  \( -size 824x824 xc:none -fill white -draw "roundrectangle 0,0 823,823 185,185" \) \
  -alpha off -compose CopyOpacity -composite "$WORK/rounded.png"
magick -size 1024x1024 xc:none \
  \( "$WORK/rounded.png" -background black -shadow 30x10+0+0 \) -geometry +80+92 -composite \
  "$WORK/rounded.png" -geometry +100+100 -composite "$WORK/app.png"
mkdir "$WORK/AppIcon.iconset"
for size in 16 32 128 256 512; do
  magick "$WORK/app.png" -resize "${size}x${size}" "PNG32:$WORK/AppIcon.iconset/icon_${size}x${size}.png"
  magick "$WORK/app.png" -resize "$((size * 2))x$((size * 2))" "PNG32:$WORK/AppIcon.iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$WORK/AppIcon.iconset" -o "$ROOT/packaging/AppIcon.icns"

# Windows: the same rounded square in every size Windows asks for (tray 16/20/24/32, Start 48+).
magick "$WORK/app.png" -define icon:auto-resize=256,64,48,40,32,24,20,16 "$ROOT/packaging/AppIcon.ico"
mkdir -p "$ROOT/packaging/windows"
for spec in StoreLogo:50 Square44x44Logo:44 Square150x150Logo:150; do
  magick "$WORK/app.png" -resize "${spec#*:}x${spec#*:}" "PNG32:$ROOT/packaging/windows/${spec%%:*}.png"
done

# The snout as a line icon, drawn rather than traced: an outline `$1` points thick with solid
# nostrils, tilted like the app icon, on a 22x18pt canvas. Drawn at 16 pixels per point into `$2`;
# averaging it down gives clean edges at any smaller scale.
K=16
pt() { awk -v v="$1" -v k="$K" 'BEGIN { printf "%g", v * k }'; }
snout() {
  local at
  at="translate $(pt 11),$(pt 9) rotate -21"
  magick -size "$(pt 22)x$(pt 18)" xc:none \
    -fill none -stroke black -strokewidth "$(pt "$1")" \
    -draw "$at roundrectangle $(pt -8.25),$(pt -5.25) $(pt 8.25),$(pt 5.25) $(pt 5.25),$(pt 5.25)" \
    -stroke none -fill black \
    -draw "$at roundrectangle $(pt -5.1),$(pt -1.15) $(pt -1.5),$(pt 1.15) $(pt 1.15),$(pt 1.15)" \
    -draw "$at roundrectangle $(pt 1.5),$(pt -1.15) $(pt 5.1),$(pt 1.15) $(pt 1.15),$(pt 1.15)" \
    "$2"
}

# Menu bar template: SF Symbols' regular weight (1.5pt), 1x and 2x.
snout 1.5 "$WORK/glyph.png"
magick "$WORK/glyph.png" -scale 12.5% "$WORK/MenuBarIcon@2x.png"
magick "$WORK/glyph.png" -scale 6.25% "$WORK/MenuBarIcon.png"
tiffutil -cathidpicheck "$WORK/MenuBarIcon.png" "$WORK/MenuBarIcon@2x.png" -out "$ROOT/packaging/MenuBarIcon.tiff" >/dev/null

# Search field icon, in place of the magnifying glass: a bit heavier (1.8pt) to match the field's
# large symbols, at 4x so it stays sharp at any field size. The density makes it 22x18pt.
snout 1.8 "$WORK/search.png"
magick "$WORK/search.png" -scale 25% -units PixelsPerInch -density 288 "PNG32:$ROOT/packaging/SearchIcon.png"

# Web. The logo is the full square (the page rounds it); the favicon brings its own corners.
magick "$SRC" -resize 180x180 -strip "$WORK/logo.png"
magick "$SRC" -resize 64x64 \
  \( -size 64x64 xc:none -fill white -draw "roundrectangle 0,0 63,63 14,14" \) \
  -alpha off -compose CopyOpacity -composite -strip "$WORK/favicon.png"
cp "$WORK/logo.png" "$ROOT/web/public/icon.png"
cp "$WORK/favicon.png" "$ROOT/web/public/favicon.png"

ls -la "$ROOT/packaging/AppIcon.icns" "$ROOT/packaging/AppIcon.ico" "$ROOT/packaging/MenuBarIcon.tiff" "$ROOT/packaging/SearchIcon.png" "$ROOT/web/public/icon.png" "$ROOT/web/public/favicon.png"
