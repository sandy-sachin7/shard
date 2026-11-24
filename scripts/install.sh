#!/usr/bin/env bash
set -euo pipefail

REPO="sandy-sachin7/shard"
BINARY="shard"

# Detect OS and architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
  linux)
    case "$ARCH" in
      x86_64)  TARGET="x86_64-linux" ;;
      aarch64|arm64) TARGET="aarch64-linux" ;;
      *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  darwin)
    case "$ARCH" in
      x86_64)  TARGET="x86_64-macos" ;;
      aarch64|arm64) TARGET="aarch64-macos" ;;
      *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "Unsupported OS: $OS" >&2
    echo "See https://github.com/$REPO/releases for manual download" >&2
    exit 1
    ;;
esac

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# Fetch latest release tag
echo "Fetching latest release..."
LATEST="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | head -n1 | cut -d'"' -f4)"
if [ -z "$LATEST" ]; then
  echo "Failed to detect latest release" >&2
  exit 1
fi

ASSET="${BINARY}-${LATEST}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/$REPO/releases/download/$LATEST/$ASSET"
CHECKSUM_URL="$DOWNLOAD_URL.sha256"

INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

echo "Downloading $BINARY $LATEST ($TARGET)..."
curl -fsSL "$DOWNLOAD_URL" -o "$TMPDIR/$ASSET"

echo "Verifying checksum..."
EXPECTED_HASH="$(curl -fsSL "$CHECKSUM_URL" | awk '{print $1}')"
if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL_HASH="$(sha256sum "$TMPDIR/$ASSET" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  ACTUAL_HASH="$(shasum -a 256 "$TMPDIR/$ASSET" | awk '{print $1}')"
else
  echo "Neither sha256sum nor shasum found; cannot verify download integrity" >&2
  exit 1
fi

if [ -z "$EXPECTED_HASH" ] || [ "$EXPECTED_HASH" != "$ACTUAL_HASH" ]; then
  echo "Checksum mismatch! Aborting installation." >&2
  echo "Expected: $EXPECTED_HASH" >&2
  echo "Actual:   $ACTUAL_HASH" >&2
  exit 1
fi

echo "Extracting..."
tar xzf "$TMPDIR/$ASSET" -C "$TMPDIR"

BIN_SRC="$TMPDIR/${BINARY}-${LATEST}-${TARGET}/${BINARY}"
if [ ! -f "$BIN_SRC" ]; then
  BIN_SRC="$TMPDIR/${BINARY}"
fi

echo "Installing to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR" 2>/dev/null || true
if [ ! -w "$INSTALL_DIR" ]; then
  echo "No write permission to $INSTALL_DIR. Trying sudo..."
  sudo install -m 0755 "$BIN_SRC" "$INSTALL_DIR/$BINARY"
else
  install -m 0755 "$BIN_SRC" "$INSTALL_DIR/$BINARY"
fi

echo ""
echo "✓ $BINARY $LATEST installed to $INSTALL_DIR/$BINARY"
echo "Run 'shard --help' to get started."
