#!/bin/sh
set -e

REPO="msaroufim/pretty-rocm-smi"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

LATEST=$(curl -sL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)

if [ -z "$LATEST" ]; then
    echo "Error: could not fetch latest release" >&2
    exit 1
fi

echo "Installing pretty-rocm-smi ${LATEST}..."

TMPFILE=$(mktemp)
curl -sL "https://github.com/${REPO}/releases/download/${LATEST}/pretty-rocm-smi" -o "$TMPFILE"
chmod +x "$TMPFILE"

if [ -w "$INSTALL_DIR" ]; then
    mv "$TMPFILE" "${INSTALL_DIR}/pretty-rocm-smi"
else
    sudo mv "$TMPFILE" "${INSTALL_DIR}/pretty-rocm-smi"
fi

echo "Installed pretty-rocm-smi to ${INSTALL_DIR}/pretty-rocm-smi"
