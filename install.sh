#!/usr/bin/env bash
set -euo pipefail

REPO="${TAIJI_LOG_REPO:-loverara/taiji-log}"
VERSION="${TAIJI_LOG_VERSION:-}"
INSTALL_DIR="${TAIJI_LOG_INSTALL_DIR:-$HOME/.local/bin}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) REPO="${2:-}"; shift 2 ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --to) INSTALL_DIR="${2:-}"; shift 2 ;;
    *) echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin) OS_ID="apple-darwin" ;;
  Linux) OS_ID="unknown-linux-gnu" ;;
  *) echo "Unsupported OS: $OS" >&2; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH_ID="x86_64" ;;
  arm64|aarch64) ARCH_ID="aarch64" ;;
  *) echo "Unsupported arch: $ARCH" >&2; exit 1 ;;
esac

TARGET="${ARCH_ID}-${OS_ID}"
ASSET="taiji-log-${TARGET}.tar.gz"

if [[ -n "$VERSION" ]]; then
  BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
else
  BASE_URL="https://github.com/${REPO}/releases/latest/download"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

ARCHIVE_PATH="${TMP_DIR}/${ASSET}"
SUMS_PATH="${TMP_DIR}/sha256sums.txt"

curl -fsSL -o "$ARCHIVE_PATH" "${BASE_URL}/${ASSET}"
curl -fsSL -o "$SUMS_PATH" "${BASE_URL}/sha256sums.txt"

EXPECTED="$(grep "  ${ASSET}$" "$SUMS_PATH" | awk '{print $1}' || true)"
if [[ -z "$EXPECTED" ]]; then
  echo "Checksum for ${ASSET} not found in sha256sums.txt" >&2
  exit 1
fi

ACTUAL=""
if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "$ARCHIVE_PATH" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  ACTUAL="$(shasum -a 256 "$ARCHIVE_PATH" | awk '{print $1}')"
elif command -v openssl >/dev/null 2>&1; then
  ACTUAL="$(openssl dgst -sha256 "$ARCHIVE_PATH" | awk '{print $2}')"
else
  echo "No sha256 tool found (sha256sum/shasum/openssl)" >&2
  exit 1
fi

if [[ "$EXPECTED" != "$ACTUAL" ]]; then
  echo "Checksum mismatch for ${ASSET}" >&2
  echo "expected: ${EXPECTED}" >&2
  echo "actual:   ${ACTUAL}" >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR"
tar -xzf "$ARCHIVE_PATH" -C "$TMP_DIR"
chmod +x "${TMP_DIR}/taiji-log"
mv "${TMP_DIR}/taiji-log" "${INSTALL_DIR}/taiji-log"

if command -v taiji-log >/dev/null 2>&1; then
  taiji-log --version >/dev/null 2>&1 || true
fi

if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
  echo "Installed to ${INSTALL_DIR}/taiji-log"
  echo "Add to PATH (zsh): echo 'export PATH=\"${INSTALL_DIR}:$PATH\"' >> ~/.zshrc && source ~/.zshrc"
else
  echo "Installed: taiji-log"
fi
