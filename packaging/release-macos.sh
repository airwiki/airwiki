#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
APP="$ROOT/target/packages/macos/AirWiki.app"
DMG="$ROOT/target/packages/macos/AirWiki_0.2.0_aarch64.dmg"
UPDATE_ARCHIVE="$ROOT/target/packages/macos/AirWiki.app.tar.gz"
NOTARY_ZIP="$ROOT/target/packages/macos/airwiki-notary.zip"
DMG_STAGE=$(mktemp -d "${TMPDIR:-/tmp}/airwiki-dmg.XXXXXX")

cleanup() {
  rm -rf -- "$DMG_STAGE"
  rm -f -- "$NOTARY_ZIP"
}
trap cleanup EXIT HUP INT TERM

: "${AIRWIKI_SIGNING_IDENTITY:?Developer ID Application identity is required}"
: "${APPLE_API_KEY_PATH:?path to the temporary App Store Connect API key is required}"
: "${APPLE_API_KEY_ID:?App Store Connect API key id is required}"
: "${APPLE_API_ISSUER_ID:?App Store Connect API issuer id is required}"
: "${CARGO_PACKAGER_SIGN_PRIVATE_KEY:?updater private key is required}"
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
rm -f -- "$DMG"
ditto "$APP" "$DMG_STAGE/AirWiki.app"
ln -s /Applications "$DMG_STAGE/Applications"
hdiutil create -fs HFS+ -format UDZO -volname "AirWiki" \
  -srcfolder "$DMG_STAGE" "$DMG"
codesign --force --sign "$AIRWIKI_SIGNING_IDENTITY" --timestamp "$DMG"
codesign --verify --strict --verbose=2 "$DMG"
notarize "$DMG"
xcrun stapler staple -v "$DMG"
xcrun stapler validate -v "$DMG"
hdiutil verify "$DMG"
spctl --assess --type open --context context:primary-signature --verbose=4 "$DMG"

# Match cargo-packager's app updater shape: the archive contains the .app root.
tar -czf "$UPDATE_ARCHIVE" -C "$(dirname "$APP")" "$(basename "$APP")"
cargo packager signer sign "$UPDATE_ARCHIVE"
test -s "$UPDATE_ARCHIVE.sig"

shasum -a 256 "$DMG" "$UPDATE_ARCHIVE" "$UPDATE_ARCHIVE.sig"
