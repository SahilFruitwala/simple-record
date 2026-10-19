#!/bin/sh
# Regenerate the logo and every icon format from assets/logo/make_logo.py:
#   support/icon.png (1024), support/AppIcon.icns (macOS), support/icon.ico (Windows)
# Needs Python with Pillow, and macOS iconutil for the .icns.
set -eu
cd "$(dirname "$0")/.."
python3 assets/logo/make_logo.py
SET=$(mktemp -d)/AppIcon.iconset
mkdir -p "$SET"
for s in 16 32 128 256 512; do
  sips -z $s $s support/icon.png --out "$SET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) support/icon.png --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$SET" -o support/AppIcon.icns
python3 - <<'PY'
from PIL import Image
im = Image.open("support/icon.png").convert("RGBA")
# Crop the transparent margin of the macOS icon grid so Windows icons fill their tile.
im = im.crop((84, 84, 940, 940))
im.save("support/icon.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
PY
echo "icons written to support/"
