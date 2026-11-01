#!/bin/sh
# Build dist/SimpleRecord.app (scripts/build.sh) and wrap it in dist/SimpleRecord-<version>-macos-arm64.dmg.
# In CI the app is signed with the self-signed cert from the SIGNING_* secrets (see
# scripts/make-signing-cert.sh); without them it is ad-hoc signed and permissions reset per update.
set -eu
cd "$(dirname "$0")/.."
REC_RELEASE=1 ./scripts/build.sh
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
DMG="dist/SimpleRecord-$VERSION-macos-arm64.dmg"
STAGE=$(mktemp -d)
cp -R dist/SimpleRecord.app "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "SimpleRecord" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
shasum -a 256 "$DMG" | tee "$DMG.sha256"
echo "packaged $DMG"
