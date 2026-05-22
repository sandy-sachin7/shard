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
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
  darwin)
    case "$ARCH" in
      x86_64)  TARGET="x86_64-macos" ;;
      aarch64|arm64) TARGET="aarch64-macos" ;;
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
  *)
    echo "Unsupported OS: $OS"
    echo "See https://github.com/$REPO/releases for manual download"
    exit 1
    ;;
esac

# Fetch latest release tag
echo "Fetching latest release..."
LATEST="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)"
if [ -z "$LATEST" ]; then
  echo "Failed to detect latest release"
  exit 1
fi

ASSET="${BINARY}-${LATEST}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/$REPO/releases/download/$LATEST/$ASSET"
CHECKSUM_URL="$DOWNLOAD_URL.sha256"

INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

echo "Downloading $BINARY $LATEST ($TARGET)..."
curl -fsSL "$DOWNLOAD_URL" -o "/tmp/$ASSET"

echo "Verifying checksum..."
EXPECTED_HASH="$(curl -fsSL "$CHECKSUM_URL" | awk '{print $1}')"
ACTUAL_HASH="$(sha256sum "/tmp/$ASSET" | awk '{print $1}')"

if [ "$EXPECTED_HASH" != "$ACTUAL_HASH" ]; then
  echo "Checksum mismatch!"
  echo "Expected: $EXPECTED_HASH"
  echo "Actual:   $ACTUAL_HASH"
  exit 1
fi

echo "Extracting..."
tar xzf "/tmp/$ASSET" -C /tmp

echo "Installing to $INSTALL_DIR..."
if [ ! -w "$INSTALL_DIR" ]; then
  echo "No write permission to $INSTALL_DIR. Trying sudo..."
  sudo mv "/tmp/${BINARY}-${LATEST}-${TARGET}/${BINARY}" "$INSTALL_DIR/"
else
  mv "/tmp/${BINARY}-${LATEST}-${TARGET}/${BINARY}" "$INSTALL_DIR/"
fi

rm -rf "/tmp/$ASSET" "/tmp/${BINARY}-${LATEST}-${TARGET}"

echo ""
echo "✓ $BINARY $LATEST installed to $INSTALL_DIR/$BINARY"
echo "Run 'shard --help' to get started."
