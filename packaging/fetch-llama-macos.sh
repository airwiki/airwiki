#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CACHE="$ROOT/target/packaging-cache"
ARCHIVE="$CACHE/llama-b9946-bin-macos-arm64.tar.gz"
DEST="$ROOT/resources/llama/macos-aarch64"
STAGE="$ROOT/target/packaging-stage/llama-macos-arm64"
URL="https://github.com/ggml-org/llama.cpp/releases/download/b9946/llama-b9946-bin-macos-arm64.tar.gz"
EXPECTED="d51d0ab59f0f44282c532449bb1d0098367e3b9429d20b8d7e7ab270eaa2393f"
SERVER_EXPECTED="12df97ffa9d48545e96cd3237a71f78efd1cc0222f971cbd65f7ab57e793b128"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "This staging script must run on macOS." >&2
  exit 1
fi

mkdir -p "$CACHE" "$(dirname -- "$STAGE")" "$(dirname -- "$DEST")"

ARCHIVE_VALID=false
if [ -f "$ARCHIVE" ]; then
  CACHED_HASH=$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')
  if [ "$CACHED_HASH" = "$EXPECTED" ]; then
    ARCHIVE_VALID=true
  fi
fi

if [ "$ARCHIVE_VALID" = false ]; then
  if ! curl --fail --location --retry 3 --continue-at - --output "$ARCHIVE" "$URL"; then
    # A corrupt file with the final byte length can receive HTTP 416 when
    # resumed. Retry once from zero rather than accepting or looping on it.
    rm -f -- "$ARCHIVE"
    curl --fail --location --retry 3 --output "$ARCHIVE" "$URL"
  fi
fi

ACTUAL=$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')
if [ "$ACTUAL" != "$EXPECTED" ]; then
  echo "Resumed archive did not verify; retrying once from zero." >&2
  rm -f -- "$ARCHIVE"
  curl --fail --location --retry 3 --output "$ARCHIVE" "$URL"
  ACTUAL=$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')
  if [ "$ACTUAL" != "$EXPECTED" ]; then
    echo "llama.cpp archive SHA-256 mismatch: expected $EXPECTED, got $ACTUAL" >&2
    exit 1
  fi
fi

case "$STAGE:$DEST" in
  "$ROOT"/target/packaging-stage/*:"$ROOT"/resources/llama/*) ;;
  *) echo "Refusing to replace an unexpected path." >&2; exit 1 ;;
esac

rm -rf -- "$STAGE"
mkdir -p "$STAGE"
tar -xzf "$ARCHIVE" -C "$STAGE"

SERVER="$STAGE/llama-b9946/llama-server"
if [ ! -f "$SERVER" ] || [ ! -x "$SERVER" ]; then
  echo "Verified archive did not contain llama-b9946/llama-server." >&2
  exit 1
fi
SERVER_ACTUAL=$(shasum -a 256 "$SERVER" | awk '{print $1}')
if [ "$SERVER_ACTUAL" != "$SERVER_EXPECTED" ]; then
  echo "llama-server SHA-256 mismatch inside verified archive." >&2
  exit 1
fi

rm -rf -- "$DEST"
mv "$STAGE" "$DEST"
echo "Staged verified llama.cpp b9946 runtime at $DEST"
