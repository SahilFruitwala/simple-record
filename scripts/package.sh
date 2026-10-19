#!/bin/sh
# Build dist/Rec.app (scripts/build.sh) and wrap it in dist/Rec-<version>-macos-arm64.dmg.
# In CI there is no signing identity, so the app is ad-hoc signed (users right-click > Open once).
set -eu
cd "$(dirname "$0")/.."
./scripts/build.sh
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
DMG="dist/Rec-$VERSION-macos-arm64.dmg"
STAGE=$(mktemp -d)
cp -R dist/Rec.app "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "Rec" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
shasum -a 256 "$DMG" | tee "$DMG.sha256"
echo "packaged $DMG"
