#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
OUT_DIR="$ROOT/target/packages/macos"
OUT_NAME="AirWiki_0.2.0_aarch64.dmg"
TAURI_BUNDLE_DIR="$ROOT/target/aarch64-apple-darwin/release/bundle"
APP="$TAURI_BUNDLE_DIR/macos/AirWiki.app"
TAURI_DMG="$TAURI_BUNDLE_DIR/dmg/$OUT_NAME"
FINAL_APP="$OUT_DIR/AirWiki.app"
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
STAGED_RUNTIME_DIR="$ROOT/target/packaging-resources/macos/llama"
PACKAGED_RUNTIME_DIR="$APP/Contents/Resources/llama"
LAUNCH_AGENT_SOURCE="$ROOT/packaging/macos/io.github.airwiki.AirWiki.background.plist"
LAUNCH_AGENT_DIR="$APP/Contents/Library/LaunchAgents"
LAUNCH_AGENT="$LAUNCH_AGENT_DIR/io.github.airwiki.AirWiki.background.plist"
SIGNING_IDENTITY=${AIRWIKI_SIGNING_IDENTITY:--}
SIGNING_PURPOSE=${AIRWIKI_SIGNING_PURPOSE:-}

if [ -z "$SIGNING_PURPOSE" ]; then
  if [ "$SIGNING_IDENTITY" = "-" ]; then
    SIGNING_PURPOSE=adhoc
  else
    SIGNING_PURPOSE=release
  fi
fi
case "$SIGNING_PURPOSE" in
  adhoc)
    if [ "$SIGNING_IDENTITY" != "-" ]; then
      echo "ad-hoc packaging must use the ad-hoc signing identity" >&2
      exit 1
    fi
    ;;
  development | release)
    if [ "$SIGNING_IDENTITY" = "-" ]; then
      echo "identified packaging requires a non-ad-hoc signing identity" >&2
      exit 1
    fi
    ;;
  *)
    echo "AIRWIKI_SIGNING_PURPOSE must be adhoc, development or release" >&2
    exit 1
    ;;
esac

# A failed build must never cause an older bundle or staged payload to survive.
rm -rf -- "$APP" "$FINAL_APP" "$STAGED_RUNTIME_DIR"
rm -f -- "$TAURI_DMG" "$OUT_DIR/$OUT_NAME" "$OUT_DIR/rw.$OUT_NAME"
rm -f -- "$SOURCE_MCPB" "$READY_STAMP"

cargo run --locked -p xtask -- licenses check
./packaging/fetch-llama-macos.sh
mkdir -p -- "$STAGED_RUNTIME_DIR"
cp -RL -- "$SOURCE_RUNTIME_DIR/." "$STAGED_RUNTIME_DIR/"
if [ -n "$(find "$STAGED_RUNTIME_DIR" -type l -print -quit)" ] ||
  ! diff -qr "$SOURCE_RUNTIME_DIR" "$STAGED_RUNTIME_DIR" >/dev/null; then
  echo "staged llama.cpp runtime differs from the verified source payload" >&2
  exit 1
fi
cargo build --locked --release --target aarch64-apple-darwin -p airwiki-mcp-bridge
./packaging/sign-macos-bridge.sh
cargo run --locked -p xtask -- mcpb build \
  --target aarch64-apple-darwin \
  --bridge "$RELEASE_BRIDGE" \
  --output "$SOURCE_MCPB"
touch "$READY_STAMP"

(
  cd "$ROOT/apps/desktop"
  export APPLE_SIGNING_IDENTITY="$SIGNING_IDENTITY"
  ./ui/node_modules/.bin/tauri build \
    --ci \
    --config ../../packaging/macos/tauri.bundle.conf.json \
    --target aarch64-apple-darwin \
    --bundles app,dmg
)

if [ ! -f "$READY_STAMP" ]; then
  echo "Tauri packaging preparation did not complete" >&2
  exit 1
fi
if [ ! -d "$APP" ]; then
  echo "Tauri bundler failed before producing AirWiki.app" >&2
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

# AssetManager verifies the pinned upstream hashes at runtime. Tauri signs only
# owned binaries and the outer application; the llama.cpp tree remains a
# resource whose exact upstream bytes must survive bundling.
if ! runtime_bytes_match; then
  echo "packaged llama.cpp runtime differs from the verified source payload" >&2
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
case "$SIGNING_PURPOSE" in
  adhoc)
    case "$SIGNATURE_DETAILS" in
      *"Signature=adhoc"*"Sealed Resources version="*) ;;
      *)
        echo "development application is not fully ad-hoc signed" >&2
        exit 1
        ;;
    esac
    ;;
  development)
    case "$SIGNATURE_DETAILS" in
      *"Authority=Apple Development:"*"TeamIdentifier="*"Runtime Version="*"Sealed Resources version="*) ;;
      *)
        echo "development application is not Apple Development signed with Hardened Runtime" >&2
        exit 1
        ;;
    esac
    ;;
  release)
    case "$SIGNATURE_DETAILS" in
      *"Authority=Developer ID Application:"*"TeamIdentifier="*"Runtime Version="*"Sealed Resources version="*) ;;
      *)
        echo "release application is not Developer ID signed with Hardened Runtime" >&2
        exit 1
        ;;
    esac
    ;;
esac

if [ ! -f "$TAURI_DMG" ]; then
  echo "Tauri bundler did not produce the expected DMG" >&2
  exit 1
fi
mkdir -p -- "$OUT_DIR"
cp -R -- "$APP" "$FINAL_APP"
cp -- "$TAURI_DMG" "$OUT_DIR/$OUT_NAME"
if ! codesign --verify --deep --strict --verbose=2 "$FINAL_APP"; then
  echo "copied application failed strict code-signature verification" >&2
  exit 1
fi
if [ "$SIGNING_IDENTITY" != "-" ]; then
  codesign --verify --strict --verbose=2 "$OUT_DIR/$OUT_NAME"
fi
if ! hdiutil verify "$OUT_DIR/$OUT_NAME"; then
  echo "packaged DMG failed integrity verification" >&2
  exit 1
fi
if ! DMG_RESOURCES=$(hdiutil udifderez -xml "$OUT_DIR/$OUT_NAME" 2>/dev/null) ||
  ! printf '%s' "$DMG_RESOURCES" | grep -q '<key>LPic</key>' ||
  ! printf '%s' "$DMG_RESOURCES" | grep -q '<key>STR#</key>' ||
  ! printf '%s' "$DMG_RESOURCES" | grep -q '<key>TEXT</key>'; then
  echo "packaged DMG does not contain the required license agreement" >&2
  exit 1
fi
