#!/usr/bin/env bash
set -euo pipefail

CRATE_NAME="url_predictor"
FEATURES="${FEATURES:-real-psl}"

# Install targets
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios

# Build static libs
cargo build --release --features "${FEATURES}" --target aarch64-apple-ios         # device (arm64)
cargo build --release --features "${FEATURES}" --target aarch64-apple-ios-sim     # sim (arm64)
cargo build --release --features "${FEATURES}" --target x86_64-apple-ios          # sim (x86_64)

# Make thin frameworks directories
rm -rf dist/ios
mkdir -p dist/ios/device/lib dist/ios/sim/lib dist/include

cp target/aarch64-apple-ios/release/lib${CRATE_NAME}.a             dist/ios/device/lib/
# Create a universal simulator staticlib (arm64 + x86_64)
lipo -create \
  target/aarch64-apple-ios-sim/release/lib${CRATE_NAME}.a \
  target/x86_64-apple-ios/release/lib${CRATE_NAME}.a \
  -output dist/ios/sim/lib/lib${CRATE_NAME}.a

# Header
if ! command -v cbindgen >/dev/null 2>&1; then cargo install cbindgen; fi
cbindgen --config cbindgen.toml --crate ${CRATE_NAME} --output dist/include/ddg_url_predictor.h

# Build XCFramework
rm -rf dist/ios/DDGUrlPredictor.xcframework
xcodebuild -create-xcframework \
  -library dist/ios/device/lib/lib${CRATE_NAME}.a -headers dist/include \
  -library dist/ios/sim/lib/lib${CRATE_NAME}.a    -headers dist/include \
  -output dist/ios/DDGUrlPredictor.xcframework

echo "✅ XCFramework at dist/ios/DDGUrlPredictor.xcframework"

