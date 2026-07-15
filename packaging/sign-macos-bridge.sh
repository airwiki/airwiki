#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BRIDGE="$ROOT/target/aarch64-apple-darwin/release/airwiki-mcp-bridge"
IDENTITY=${AIRWIKI_SIGNING_IDENTITY:--}

if [ ! -x "$BRIDGE" ]; then
  echo "fresh macOS MCP bridge is missing" >&2
  exit 1
fi

codesign --force --sign "$IDENTITY" --options runtime --timestamp "$BRIDGE"
codesign --verify --strict --verbose=2 "$BRIDGE"
