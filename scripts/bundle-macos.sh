#!/usr/bin/env bash
# Build app-macos in release and assemble a macOS .app bundle.
#
# Output: target/release/bundle/svg3-macos.app
# No external tools beyond `cargo` and coreutils. Idempotent.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

APP_NAME="svg3-macos"
BIN_NAME="svg3-macos"
TARGET_DIR="target/release"
BUNDLE_DIR="${TARGET_DIR}/bundle/${APP_NAME}.app"
PLIST_SRC="app-macos/macos/Info.plist"

echo "==> building ${BIN_NAME} (release)"
cargo build --release -p app-macos

echo "==> assembling ${BUNDLE_DIR}"
rm -rf "${BUNDLE_DIR}"
mkdir -p "${BUNDLE_DIR}/Contents/MacOS"
mkdir -p "${BUNDLE_DIR}/Contents/Resources"

cp "${TARGET_DIR}/${BIN_NAME}" "${BUNDLE_DIR}/Contents/MacOS/${BIN_NAME}"
chmod +x "${BUNDLE_DIR}/Contents/MacOS/${BIN_NAME}"
cp "${PLIST_SRC}" "${BUNDLE_DIR}/Contents/Info.plist"
printf 'APPL????' > "${BUNDLE_DIR}/Contents/PkgInfo"

echo "==> done: ${BUNDLE_DIR}"
echo "    run: open ${BUNDLE_DIR}"
