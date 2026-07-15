#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
APP="$ROOT/target/packages/macos/AirWiki.app"
OUT_DIR="$ROOT/target/packages/macos"
OUT_NAME="AirWiki_0.2.0_aarch64.dmg"
RELEASE_BINARY="$ROOT/target/aarch64-apple-darwin/release/airwiki"
PACKAGED_BINARY="$APP/Contents/MacOS/airwiki"
RELEASE_BRIDGE="$ROOT/target/aarch64-apple-darwin/release/airwiki-mcp-bridge"
PACKAGED_BRIDGE="$APP/Contents/Resources/integrations/bridge/airwiki-mcp-bridge"
SOURCE_MCPB="$ROOT/target/mcpb/aarch64-apple-darwin/airwiki-claude.mcpb"
PACKAGED_MCPB="$APP/Contents/Resources/integrations/airwiki-claude.mcpb"
SOURCE_ICON="$ROOT/resources/branding/airwiki.icns"
PACKAGED_ICON="$APP/Contents/Resources/airwiki.icns"
READY_STAMP="$ROOT/target/packaging-macos-ready.stamp"
SOURCE_RUNTIME_DIR="$ROOT/resources/llama/macos-aarch64"
PACKAGED_RUNTIME_DIR="$APP/Contents/Resources/llama"
LAUNCH_AGENT_SOURCE="$ROOT/packaging/macos/io.github.airwiki.AirWiki.background.plist"
LAUNCH_AGENT_DIR="$APP/Contents/Library/LaunchAgents"
LAUNCH_AGENT="$LAUNCH_AGENT_DIR/io.github.airwiki.AirWiki.background.plist"
SIGNING_IDENTITY=${AIRWIKI_SIGNING_IDENTITY:--}

# A failed build must never cause the fallback to package an older bundle.
rm -rf -- "$APP"
rm -f -- "$OUT_DIR/$OUT_NAME" "$OUT_DIR/rw.$OUT_NAME"
rm -f -- "$SOURCE_MCPB" "$READY_STAMP"

cargo packager --config packaging/macos/Packager.toml || true

if [ ! -f "$READY_STAMP" ]; then
  echo "cargo-packager did not complete its build and validation hook" >&2
  exit 1
fi
if [ ! -d "$APP" ]; then
  echo "cargo-packager failed before producing AirWiki.app" >&2
  exit 1
fi
if [ ! -x "$RELEASE_BINARY" ] || [ ! -x "$PACKAGED_BINARY" ] ||
  [ ! -x "$RELEASE_BRIDGE" ] || [ ! -x "$PACKAGED_BRIDGE" ]; then
  echo "fresh release or packaged application binary is missing" >&2
  exit 1
fi
if [ ! -f "$SOURCE_MCPB" ] || [ ! -f "$PACKAGED_MCPB" ]; then
  echo "fresh or packaged Claude MCPB is missing" >&2
  exit 1
fi
if [ ! -f "$SOURCE_ICON" ] || [ ! -f "$PACKAGED_ICON" ]; then
  echo "source or packaged application icon is missing" >&2
  exit 1
fi
if [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIconFile' "$APP/Contents/Info.plist")" != \
  "airwiki.icns" ]; then
  echo "application bundle does not reference the AirWiki icon" >&2
  exit 1
fi
if ! cmp -s "$SOURCE_ICON" "$PACKAGED_ICON"; then
  echo "packaged application icon differs from its source" >&2
  exit 1
fi

# SMAppService only accepts launch-agent definitions sealed inside the app
# bundle. This build-time copy never writes to the user's LaunchAgents folder.
mkdir -p -- "$LAUNCH_AGENT_DIR"
cp -- "$LAUNCH_AGENT_SOURCE" "$LAUNCH_AGENT"
chmod 0644 "$LAUNCH_AGENT"
if ! cmp -s "$LAUNCH_AGENT_SOURCE" "$LAUNCH_AGENT"; then
  echo "packaged launch agent differs from its source" >&2
  exit 1
fi

# The outer-bundle signing step below changes the executable's signature
# envelope. Architecture plus the linker's UUID identifies the fresh build
# without treating post-signing bytes as stable.
if ! RELEASE_ARCH=$(xcrun lipo -archs "$RELEASE_BINARY") ||
  ! PACKAGED_ARCH=$(xcrun lipo -archs "$PACKAGED_BINARY"); then
  echo "could not inspect packaged application architecture" >&2
  exit 1
fi
if [ "$RELEASE_ARCH" != "arm64" ] || [ "$PACKAGED_ARCH" != "arm64" ]; then
  echo "fresh release and packaged application must both be arm64" >&2
  exit 1
fi

mach_uuid_arm64() {
  xcrun dwarfdump --uuid "$1" | awk '
    $1 == "UUID:" && $3 == "(arm64)" { count += 1; uuid = $2 }
    END {
      if (count != 1 || length(uuid) != 36) exit 1
      print uuid
    }
  '
}

if ! RELEASE_UUID=$(mach_uuid_arm64 "$RELEASE_BINARY") ||
  ! PACKAGED_UUID=$(mach_uuid_arm64 "$PACKAGED_BINARY"); then
  echo "could not inspect packaged application Mach-O UUID" >&2
  exit 1
fi
if [ "$RELEASE_UUID" != "$PACKAGED_UUID" ]; then
  echo "packaged application does not contain the freshly built release" >&2
  exit 1
fi

if ! RELEASE_BRIDGE_ARCH=$(xcrun lipo -archs "$RELEASE_BRIDGE") ||
  ! PACKAGED_BRIDGE_ARCH=$(xcrun lipo -archs "$PACKAGED_BRIDGE"); then
  echo "could not inspect MCP bridge architecture" >&2
  exit 1
fi
if [ "$RELEASE_BRIDGE_ARCH" != "arm64" ] || [ "$PACKAGED_BRIDGE_ARCH" != "arm64" ]; then
  echo "fresh and packaged MCP bridges must both be arm64" >&2
  exit 1
fi
if ! RELEASE_BRIDGE_UUID=$(mach_uuid_arm64 "$RELEASE_BRIDGE") ||
  ! PACKAGED_BRIDGE_UUID=$(mach_uuid_arm64 "$PACKAGED_BRIDGE"); then
  echo "could not inspect MCP bridge Mach-O UUID" >&2
  exit 1
fi
if [ "$RELEASE_BRIDGE_UUID" != "$PACKAGED_BRIDGE_UUID" ]; then
  echo "packaged application does not contain the freshly built MCP bridge" >&2
  exit 1
fi
if ! codesign --verify --strict --verbose=2 "$PACKAGED_BRIDGE"; then
  echo "packaged MCP bridge is not signed correctly" >&2
  exit 1
fi
if ! cargo run --locked -p xtask -- mcpb verify \
  --target aarch64-apple-darwin \
  --bridge "$RELEASE_BRIDGE" \
  --output "$PACKAGED_MCPB"; then
  echo "packaged Claude MCPB failed validation" >&2
  exit 1
fi
if [ "$(shasum -a 256 "$SOURCE_MCPB" | awk '{print $1}')" != \
  "$(shasum -a 256 "$PACKAGED_MCPB" | awk '{print $1}')" ]; then
  echo "packaged Claude MCPB differs from the fresh artifact" >&2
  exit 1
fi

runtime_bytes_match() {
  # The pinned upstream archive contains dylib aliases as symlinks. Packager
  # materializes those aliases as regular files; diff compares their resolved
  # bytes while the packaged side remains symlink-free.
  [ -d "$SOURCE_RUNTIME_DIR" ] &&
    [ -d "$PACKAGED_RUNTIME_DIR" ] &&
    [ -z "$(find "$PACKAGED_RUNTIME_DIR" -type l -print -quit)" ] &&
    diff -qr "$SOURCE_RUNTIME_DIR" "$PACKAGED_RUNTIME_DIR" >/dev/null
}

# AssetManager verifies the pinned upstream hashes at runtime. Signing nested
# llama.cpp Mach-O files would mutate those trusted bytes, so seal them as
# resources and sign only the outer application bundle.
if ! runtime_bytes_match; then
  echo "packaged llama.cpp runtime differs from the verified source payload" >&2
  exit 1
fi
if ! codesign --force --sign "$SIGNING_IDENTITY" --options runtime --timestamp "$APP"; then
  echo "could not sign the application bundle" >&2
  exit 1
fi
if ! runtime_bytes_match; then
  echo "application signing changed the verified llama.cpp runtime" >&2
  exit 1
fi

if [ ! -f "$APP/Contents/_CodeSignature/CodeResources" ]; then
  echo "packaged application has no sealed resource signature" >&2
  exit 1
fi
if ! codesign --verify --deep --strict --verbose=2 "$APP"; then
  echo "packaged application failed strict code-signature verification" >&2
  exit 1
fi
if ! codesign --verify --strict --verbose=2 "$PACKAGED_BRIDGE"; then
  echo "outer application signing invalidated the MCP bridge" >&2
  exit 1
fi
if ! SIGNATURE_DETAILS=$(codesign -dv --verbose=4 "$APP" 2>&1); then
  echo "could not inspect packaged application signature" >&2
  exit 1
fi
if [ "$SIGNING_IDENTITY" = "-" ]; then
  case "$SIGNATURE_DETAILS" in
    *"Signature=adhoc"*"Sealed Resources version="*) ;;
    *)
      echo "development application is not fully ad-hoc signed" >&2
      exit 1
      ;;
  esac
else
  case "$SIGNATURE_DETAILS" in
    *"Authority=Developer ID Application:"*"TeamIdentifier="*"Runtime Version="*) ;;
    *)
      echo "release application is not Developer ID signed with Hardened Runtime" >&2
      exit 1
      ;;
  esac
fi

# cargo-packager creates the .app before invoking Finder for DMG cosmetics. Its
# DMG may therefore be missing on headless runs and, when present, predates the
# outer-bundle signature above. Always rebuild it from the verified .app using
# the helper's supported non-interactive path.
CREATE_DMG=$(find "$HOME/Library/Caches/.cargo-packager" -type f -path '*/script/create-dmg' -print -quit 2>/dev/null || true)
if [ -z "$CREATE_DMG" ]; then
  echo "cargo-packager did not install its create-dmg helper" >&2
  exit 1
fi
rm -f -- "$OUT_DIR/$OUT_NAME" "$OUT_DIR/rw.$OUT_NAME"

cd "$OUT_DIR"
"$CREATE_DMG" \
  --skip-jenkins \
  --volname "AirWiki" \
  --app-drop-link 480 210 \
  --window-size 660 420 \
  --hide-extension "AirWiki.app" \
  --eula "$ROOT/LICENSE" \
  "$OUT_NAME" \
  "AirWiki.app"

if [ ! -f "$OUT_DIR/$OUT_NAME" ]; then
  echo "packaging did not produce the expected DMG" >&2
  exit 1
fi
if [ "$SIGNING_IDENTITY" != "-" ]; then
  codesign --force --sign "$SIGNING_IDENTITY" --timestamp "$OUT_DIR/$OUT_NAME"
  codesign --verify --strict --verbose=2 "$OUT_DIR/$OUT_NAME"
fi
if ! hdiutil verify "$OUT_DIR/$OUT_NAME"; then
  echo "packaged DMG failed integrity verification" >&2
  exit 1
fi
