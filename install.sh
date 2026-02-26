#!/usr/bin/env bash
set -euo pipefail

BINARY_NAME="casciit"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

echo "Building cascii-terminal-decorator (release)..."
cargo build --release

SOURCE="target/release/cascii-terminal-decorator"
if [ ! -f "$SOURCE" ]; then
    echo "Error: build artifact not found at $SOURCE"
    exit 1
fi

echo "Installing ${BINARY_NAME} to ${INSTALL_DIR}..."
if [ -w "$INSTALL_DIR" ]; then
    install -m 755 "$SOURCE" "${INSTALL_DIR}/${BINARY_NAME}"
else
    echo "Need sudo to write to ${INSTALL_DIR}"
    sudo install -m 755 "$SOURCE" "${INSTALL_DIR}/${BINARY_NAME}"
fi

echo "Done. You can now run: ${BINARY_NAME} /path-to-frames-folder"
