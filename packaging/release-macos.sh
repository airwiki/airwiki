#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -n "${AIRWIKI_RELEASE_VERSION:-}" ]; then
  VERSION=$(node "$ROOT/packaging/release-version.mjs" --expect "$AIRWIKI_RELEASE_VERSION")
else
  VERSION=$(node "$ROOT/packaging/release-version.mjs")
fi
APP="$ROOT/target/packages/macos/AirWiki.app"
DMG="$ROOT/target/packages/macos/AirWiki_${VERSION}_aarch64.dmg"
UPDATE_ARCHIVE="$ROOT/target/packages/macos/AirWiki.app.tar.gz"
NOTARY_ZIP="$ROOT/target/packages/macos/airwiki-notary.zip"
DMG_RESOURCES="$ROOT/target/packages/macos/airwiki-dmg-resources.plist"
DMG_STAGE=$(mktemp -d "${TMPDIR:-/tmp}/airwiki-dmg.XXXXXX")

cleanup() {
  rm -rf -- "$DMG_STAGE"
  rm -f -- "$NOTARY_ZIP" "$DMG_RESOURCES"
}
trap cleanup EXIT HUP INT TERM

: "${AIRWIKI_SIGNING_IDENTITY:?Developer ID Application identity is required}"
: "${APPLE_API_KEY_PATH:?path to the temporary App Store Connect API key is required}"
: "${APPLE_API_KEY_ID:?App Store Connect API key id is required}"
: "${APPLE_API_ISSUER_ID:?App Store Connect API issuer id is required}"
: "${TAURI_SIGNING_PRIVATE_KEY:?Tauri updater private key is required}"
: "${AIRWIKI_UPDATE_ENDPOINT:?compiled updater endpoint is required}"
: "${AIRWIKI_UPDATER_PUBLIC_KEY:?compiled updater public key is required}"

case "$AIRWIKI_UPDATE_ENDPOINT" in
  https://github.com/airwiki/airwiki/releases/latest/download/latest.json) ;;
  *)
    echo "release updater endpoint must be AirWiki's stable GitHub Releases manifest" >&2
    exit 1
    ;;
esac

case "$AIRWIKI_SIGNING_IDENTITY" in
  "-"|"")
    echo "release signing identity must be a Developer ID Application identity" >&2
    exit 1
    ;;
esac

notarize() {
  xcrun notarytool submit "$1" \
    --key "$APPLE_API_KEY_PATH" \
    --key-id "$APPLE_API_KEY_ID" \
    --issuer "$APPLE_API_ISSUER_ID" \
    --wait
}

cd "$ROOT"
./packaging/package-macos.sh
cargo run --locked -p xtask -- packaging verify-updater-embedded-key \
  --binary "$APP/Contents/MacOS/airwiki"

# Notarize and staple the app independently because the updater distributes an
# app archive rather than the DMG container.
rm -f -- "$NOTARY_ZIP" "$UPDATE_ARCHIVE" "$UPDATE_ARCHIVE.sig"
ditto -c -k --keepParent "$APP" "$NOTARY_ZIP"
notarize "$NOTARY_ZIP"
xcrun stapler staple -v "$APP"
xcrun stapler validate -v "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
spctl --assess --type execute --verbose=4 "$APP"

# Recreate the DMG after stapling so its app payload is byte-for-byte final.
hdiutil udifderez -xml "$DMG" >"$DMG_RESOURCES"
if ! grep -q '<key>LPic</key>' "$DMG_RESOURCES" ||
  ! grep -q '<key>STR#</key>' "$DMG_RESOURCES" ||
  ! grep -q '<key>TEXT</key>' "$DMG_RESOURCES"; then
  echo "Tauri DMG does not contain the required license agreement resources" >&2
  exit 1
fi
rm -f -- "$DMG"
ditto "$APP" "$DMG_STAGE/AirWiki.app"
ln -s /Applications "$DMG_STAGE/Applications"
hdiutil create -fs HFS+ -format UDZO -volname "AirWiki" \
  -srcfolder "$DMG_STAGE" "$DMG"
hdiutil udifrez "$DMG" -xml "$DMG_RESOURCES" -replaceall
codesign --force --sign "$AIRWIKI_SIGNING_IDENTITY" --timestamp "$DMG"
codesign --verify --strict --verbose=2 "$DMG"
notarize "$DMG"
xcrun stapler staple -v "$DMG"
xcrun stapler validate -v "$DMG"
hdiutil verify "$DMG"
spctl --assess --type open --context context:primary-signature --verbose=4 "$DMG"
FINAL_DMG_RESOURCES=$(hdiutil udifderez -xml "$DMG" 2>/dev/null)
if ! printf '%s' "$FINAL_DMG_RESOURCES" | grep -q '<key>LPic</key>' ||
  ! printf '%s' "$FINAL_DMG_RESOURCES" | grep -q '<key>STR#</key>' ||
  ! printf '%s' "$FINAL_DMG_RESOURCES" | grep -q '<key>TEXT</key>'; then
  echo "final notarized DMG lost its required license agreement resources" >&2
  exit 1
fi

# Tauri's macOS updater artifact contains the final stapled .app root.
tar -czf "$UPDATE_ARCHIVE" -C "$(dirname "$APP")" "$(basename "$APP")"
pnpm --dir apps/desktop/ui exec tauri signer sign "$UPDATE_ARCHIVE"
test -s "$UPDATE_ARCHIVE.sig"
cargo run --locked -p xtask -- packaging verify-updater-signature \
  --artifact "$UPDATE_ARCHIVE" \
  --signature "$UPDATE_ARCHIVE.sig"

shasum -a 256 "$DMG" "$UPDATE_ARCHIVE" "$UPDATE_ARCHIVE.sig"
