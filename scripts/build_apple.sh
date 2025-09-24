#!/usr/bin/env bash
set -euo pipefail

CRATE_NAME="url_predictor"
FEATURES="${FEATURES:-real-psl}"

NAME=URLPredictorRust             # Framework & Swift module name
CRATE_LIB=liburl_predictor.a      # Rust staticlib filename
MIN_MACOS=11.3
MIN_IOS=15.0

DIST_DIR="dist/apple"

# Clean output folders
rm -rf "${DIST_DIR}/macos-apple" "${DIST_DIR}/ios-apple" "${DIST_DIR}/iossim-apple"

# Ensure targets
rustup target add aarch64-apple-darwin x86_64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios

# Build all
MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS" cargo build --release \
  --config profile.release.debug=true \
  --features "${FEATURES}" \
  --target aarch64-apple-darwin

MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS" cargo build --release \
  --config profile.release.debug=true \
  --features "${FEATURES}" \
  --target x86_64-apple-darwin

IPHONEOS_DEPLOYMENT_TARGET="$MIN_IOS" cargo build --release \
  --config profile.release.debug=true \
  --features "${FEATURES}" \
  --target aarch64-apple-ios

IPHONEOS_DEPLOYMENT_TARGET="$MIN_IOS" cargo build --release \
  --config profile.release.debug=true \
  --features "${FEATURES}" \
  --target aarch64-apple-ios-sim

IPHONEOS_DEPLOYMENT_TARGET="$MIN_IOS" cargo build --release \
  --config profile.release.debug=true \
  --features "${FEATURES}" \
  --target x86_64-apple-ios

# Header
INCLUDE_DIR_ROOT="${DIST_DIR}/include"
INCLUDE_DIR="${INCLUDE_DIR_ROOT}/URLPredictorRust"
mkdir -p "$INCLUDE_DIR"
if ! command -v cbindgen >/dev/null 2>&1; then cargo install cbindgen; fi
cbindgen --config cbindgen.toml --crate ${CRATE_NAME} --output "${INCLUDE_DIR}/ddg_url_predictor.h"
cat > "${INCLUDE_DIR}/module.modulemap" <<-EOF
module URLPredictorRust {
  header "ddg_url_predictor.h"
  export *
}
EOF

## No framework assembly needed for static XCFrameworks

mkdir -p "${DIST_DIR}/macos-arm64_x86_64"
lipo -create \
  target/x86_64-apple-darwin/release/${CRATE_LIB} \
  target/aarch64-apple-darwin/release/${CRATE_LIB} \
  -output "${DIST_DIR}/macos-arm64_x86_64/${CRATE_LIB}"

mkdir -p "${DIST_DIR}/ios-arm64_x86_64-simulator"
lipo -create \
  target/x86_64-apple-ios/release/${CRATE_LIB} \
  target/aarch64-apple-ios-sim/release/${CRATE_LIB} \
  -output "${DIST_DIR}/ios-arm64_x86_64-simulator/${CRATE_LIB}"

mkdir -p "${DIST_DIR}/ios-arm64"
cp -f "target/aarch64-apple-ios/release/${CRATE_LIB}" "${DIST_DIR}/ios-arm64/${CRATE_LIB}"

# Create the xcframework directly from static libraries and headers
rm -rf "${DIST_DIR}/${NAME}.xcframework"
xcodebuild -create-xcframework \
  -library "${DIST_DIR}/macos-arm64_x86_64/${CRATE_LIB}" -headers "${INCLUDE_DIR_ROOT}" \
  -library "${DIST_DIR}/ios-arm64/${CRATE_LIB}" -headers "${INCLUDE_DIR_ROOT}" \
  -library "${DIST_DIR}/ios-arm64_x86_64-simulator/${CRATE_LIB}" -headers "${INCLUDE_DIR_ROOT}" \
  -output "${DIST_DIR}/${NAME}.xcframework"

echo "✅ Built ${DIST_DIR}/${NAME}.xcframework"

ditto -c -k --keepParent "${DIST_DIR}/${NAME}.xcframework" "${DIST_DIR}/${NAME}.xcframework.zip"

echo "✅ Zipped ${DIST_DIR}/${NAME}.xcframework.zip"

checksum="$(swift package compute-checksum "${DIST_DIR}/${NAME}.xcframework.zip")"
echo "Checksum: ${checksum}"
