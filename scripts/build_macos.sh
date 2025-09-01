#!/usr/bin/env bash
set -euo pipefail

CRATE_NAME="url_predictor"
FEATURES="${FEATURES:-real-psl}"

# Ensure targets
rustup target add x86_64-apple-darwin aarch64-apple-darwin

# Build both slices
cargo build --release --features "${FEATURES}" --target x86_64-apple-darwin
cargo build --release --features "${FEATURES}" --target aarch64-apple-darwin

# Create dist layout
rm -rf dist/macos
mkdir -p dist/macos/lib dist/include

# Lipo into a universal dylib
lipo -create \
  target/x86_64-apple-darwin/release/lib${CRATE_NAME}.dylib \
  target/aarch64-apple-darwin/release/lib${CRATE_NAME}.dylib \
  -output dist/macos/lib/lib${CRATE_NAME}.dylib

# Header
if ! command -v cbindgen >/dev/null 2>&1; then cargo install cbindgen; fi
cbindgen --config cbindgen.toml --crate ${CRATE_NAME} --output dist/include/ddg_url_predictor.h

echo "✅ macOS universal dylib at dist/macos/lib/lib${CRATE_NAME}.dylib"
echo "✅ Header at dist/include/ddg_url_predictor.h"

