#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: verify-macos-release.sh RELEASE_DIRECTORY" >&2
  exit 2
fi

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
RELEASE_DIR=$(CDPATH= cd -- "$1" && pwd)
if [ -n "${AIRWIKI_RELEASE_VERSION:-}" ]; then
  VERSION=$(node "$ROOT/packaging/release-version.mjs" --expect "$AIRWIKI_RELEASE_VERSION")
else
  VERSION=$(node "$ROOT/packaging/release-version.mjs")
fi
DMG="$RELEASE_DIR/AirWiki_${VERSION}_aarch64.dmg"
UPDATE_ARCHIVE="$RELEASE_DIR/AirWiki.app.tar.gz"
UPDATE_SIGNATURE="$RELEASE_DIR/AirWiki.app.tar.gz.sig"
EXTRACT_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/airwiki-release-verify.XXXXXX")
MOUNT_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/airwiki-release-mount.XXXXXX")
MOUNTED=false

cleanup() {
  if [ "$MOUNTED" = true ]; then
    hdiutil detach "$MOUNT_ROOT" >/dev/null 2>&1 || true
  fi
  rm -rf -- "$EXTRACT_ROOT" "$MOUNT_ROOT"
}
trap cleanup EXIT HUP INT TERM

: "${AIRWIKI_UPDATER_PUBLIC_KEY:?updater public key is required}"
: "${AIRWIKI_MACOS_TEAM_ID:?expected Apple Developer team id is required}"

python3 - "$RELEASE_DIR" "$(basename "$DMG")" \
  "$(basename "$UPDATE_ARCHIVE")" "$(basename "$UPDATE_SIGNATURE")" <<'PY'
import sys
from pathlib import Path

root = Path(sys.argv[1])
expected = set(sys.argv[2:])
entries = list(root.iterdir())
actual = {entry.name for entry in entries}
if actual != expected:
    raise ValueError("macOS release directory does not contain the exact artifact set")
for entry in entries:
    if entry.is_symlink() or not entry.is_file():
        raise ValueError(f"macOS release artifact is not a regular file: {entry.name}")
PY

cd "$ROOT"
cargo run --locked -p xtask -- packaging verify-updater-signature \
  --artifact "$UPDATE_ARCHIVE" \
  --signature "$UPDATE_SIGNATURE"

python3 - "$UPDATE_ARCHIVE" <<'PY'
import posixpath
import sys
import tarfile
from pathlib import PurePosixPath

archive = sys.argv[1]
with tarfile.open(archive, "r:gz") as bundle:
    members = bundle.getmembers()
    if not members:
        raise ValueError("macOS updater archive is empty")
    for member in members:
        path = PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts or not path.parts or path.parts[0] != "AirWiki.app":
            raise ValueError("macOS updater archive contains a path outside AirWiki.app")
        if member.isdev() or member.isfifo():
            raise ValueError("macOS updater archive contains a special file")
        if member.issym() or member.islnk():
            target = PurePosixPath(posixpath.normpath(str(path.parent / member.linkname)))
            if target.is_absolute() or ".." in target.parts or not target.parts or target.parts[0] != "AirWiki.app":
                raise ValueError("macOS updater archive contains an escaping link")
PY

tar -xzf "$UPDATE_ARCHIVE" -C "$EXTRACT_ROOT"
APP="$EXTRACT_ROOT/AirWiki.app"
if [ ! -d "$APP" ]; then
  echo "macOS updater archive does not contain AirWiki.app" >&2
  exit 1
fi

verify_app() {
  CANDIDATE=$1
  python3 "$ROOT/packaging/macos_bundle_metadata.py" \
    --application "$CANDIDATE" \
    --version "$VERSION"
  codesign --verify --deep --strict --verbose=2 "$CANDIDATE"
  xcrun stapler validate -v "$CANDIDATE"
  spctl --assess --type execute --verbose=4 "$CANDIDATE"
  DETAILS=$(codesign -dv --verbose=4 "$CANDIDATE" 2>&1)
  case "$DETAILS" in
    *"Authority=Developer ID Application:"*"TeamIdentifier=$AIRWIKI_MACOS_TEAM_ID"*"Runtime Version="*) ;;
    *)
      echo "macOS application does not match the approved Developer ID team and Hardened Runtime" >&2
      exit 1
      ;;
  esac
  ARCHS=$(lipo -archs "$CANDIDATE/Contents/MacOS/airwiki")
  if [ "$ARCHS" != "arm64" ]; then
    echo "macOS application is not an arm64-only release" >&2
    exit 1
  fi
}

verify_app "$APP"
codesign --verify --strict --verbose=2 "$DMG"
xcrun stapler validate -v "$DMG"
hdiutil verify "$DMG"
spctl --assess --type open --context context:primary-signature --verbose=4 "$DMG"
DMG_RESOURCES=$(hdiutil udifderez -xml "$DMG" 2>/dev/null)
if ! printf '%s' "$DMG_RESOURCES" | grep -q '<key>LPic</key>' ||
  ! printf '%s' "$DMG_RESOURCES" | grep -q '<key>STR#</key>' ||
  ! printf '%s' "$DMG_RESOURCES" | grep -q '<key>TEXT</key>'; then
  echo "release DMG does not contain the required license agreement resources" >&2
  exit 1
fi
hdiutil attach -nobrowse -readonly -mountpoint "$MOUNT_ROOT" "$DMG" >/dev/null
MOUNTED=true
verify_app "$MOUNT_ROOT/AirWiki.app"
if ! diff -qr "$APP" "$MOUNT_ROOT/AirWiki.app" >/dev/null; then
  echo "DMG and updater archive contain different application bytes" >&2
  exit 1
fi

shasum -a 256 "$DMG" "$UPDATE_ARCHIVE" "$UPDATE_SIGNATURE"
