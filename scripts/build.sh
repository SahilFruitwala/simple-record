#!/bin/sh
# Build rec, assemble dist/SimpleRecord.app, and code-sign it with a stable identity so
# macOS privacy grants (screen/mic/camera) survive rebuilds.
set -eu
cd "$(dirname "$0")/.."
PROFILE="${PROFILE:-release}"
# The repo is synced between machines (Mutagen): keep build output outside it.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/simple-record-target}"
if [ "$PROFILE" = release ]; then cargo build --release; else cargo build; fi
APP=dist/SimpleRecord.app
STAGE=dist/.SimpleRecord.app.new
# Build the bundle beside the old one and swap it in atomically: overwriting a
# running binary in place makes macOS kill an in-progress recording.
rm -rf "$STAGE"
mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
cp support/Info.plist "$STAGE/Contents/Info.plist"
# The bundle's version comes from Cargo.toml; the build number from CI (or 1).
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" -c "Set :CFBundleVersion ${BUILD_NUMBER:-1}" "$STAGE/Contents/Info.plist"
cp support/AppIcon.icns "$STAGE/Contents/Resources/AppIcon.icns"
cp THIRD_PARTY_NOTICES.md LICENSE-MIT LICENSE-APACHE "$STAGE/Contents/Resources/"
cp "$CARGO_TARGET_DIR/$PROFILE/simple-record" "$STAGE/Contents/MacOS/simple-record"
IDENTITY="${REC_SIGN_IDENTITY:-$(security find-identity -v -p codesigning | awk -F'"' '/Apple Development|Developer ID Application/ {print $2; exit}')}"
if [ -z "$IDENTITY" ]; then
  echo "warning: no signing identity found; ad-hoc signing (permissions reset on every rebuild)" >&2
  IDENTITY=-
fi
codesign --force --sign "$IDENTITY" --identifier com.heyfabrika.rec "$STAGE"
codesign --verify --strict "$STAGE"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$STAGE/Contents/Resources/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns.new" && mv -f "$APP/Contents/Resources/AppIcon.icns.new" "$APP/Contents/Resources/AppIcon.icns"
cp "$STAGE/Contents/Info.plist" "$APP/Contents/Info.plist.new" && mv -f "$APP/Contents/Info.plist.new" "$APP/Contents/Info.plist"
for f in THIRD_PARTY_NOTICES.md LICENSE-MIT LICENSE-APACHE; do
  cp "$STAGE/Contents/Resources/$f" "$APP/Contents/Resources/$f"
done
mv -f "$STAGE/Contents/MacOS/simple-record" "$APP/Contents/MacOS/simple-record"
rm -f "$APP/Contents/MacOS/rec"  # pre-rename executable
rm -rf "$APP/Contents/_CodeSignature"
mv "$STAGE/Contents/_CodeSignature" "$APP/Contents/_CodeSignature"
rm -rf "$STAGE"
codesign --verify --strict "$APP"
# Re-register the bundle so Finder and the Dock pick up icon/Info.plist
# changes (LaunchServices caches them per bundle path).
touch "$APP"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP" || true
ln -sf "SimpleRecord.app/Contents/MacOS/simple-record" dist/simple-record
ln -sf simple-record dist/rec  # short alias
echo "built $APP (signed: $IDENTITY)"
echo "run:   dist/simple-record --help"
