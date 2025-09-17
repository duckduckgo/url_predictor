#!/usr/bin/env bash
set -euo pipefail

CRATE_NAME="url_predictor"
FEATURES="${FEATURES:-real-psl}"

NAME=URLPredictorRust                    # Framework & Swift module name
CRATE_LIB=liburl_predictor.dylib      # Rust cdylib filename
MIN_MACOS=11.3
MIN_IOS=15.0

DIST_DIR="dist/apple"

# Ensure targets
rustup target add aarch64-apple-darwin x86_64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios

# Build all
MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS" cargo build --release --features "${FEATURES}" --target aarch64-apple-darwin
MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS" cargo build --release --features "${FEATURES}" --target x86_64-apple-darwin
IPHONEOS_DEPLOYMENT_TARGET="$MIN_IOS" cargo build --release --features "${FEATURES}" --target aarch64-apple-ios
IPHONEOS_DEPLOYMENT_TARGET="$MIN_IOS" cargo build --release --features "${FEATURES}" --target aarch64-apple-ios-sim
IPHONEOS_DEPLOYMENT_TARGET="$MIN_IOS" cargo build --release --features "${FEATURES}" --target x86_64-apple-ios

# Header
INCLUDE_DIR="${DIST_DIR}/include"
mkdir -p "$INCLUDE_DIR"
if ! command -v cbindgen >/dev/null 2>&1; then cargo install cbindgen; fi
cbindgen --config cbindgen.toml --crate ${CRATE_NAME} --output "${INCLUDE_DIR}/ddg_url_predictor.h"
cat > "${INCLUDE_DIR}/module.modulemap" <<-EOF
framework module URLPredictorRust {
  umbrella header "ddg_url_predictor.h"
  export *
}
EOF

# Helper to assemble a .framework bundle
make_framework () {
  local archdir="$1"         # e.g., target/aarch64-apple-darwin/release
  local platform="$2"        # e.g., macos, ios, ios-sim
  local arch
  arch="${archdir#*/}"       # e.g. target/aarch64-apple-darwin/release -> aarch64-apple-darwin/release
  arch="${arch%%-*}"         # e.g. aarch64-apple-darwin/release -> aarch64
  local outdir="${DIST_DIR}/${platform}-${arch}"  # unique
  local dylib="${archdir}/${CRATE_LIB}"
  # echo "Making framework in ${outdir}"

  [[ -f "$dylib" ]] || { echo "Missing $dylib"; exit 1; }

  rm -rf "${outdir}/${NAME}.framework"
  mkdir -p "${outdir}/${NAME}.framework/Headers"
  mkdir -p "${outdir}/${NAME}.framework/Modules"

  # Binary inside framework MUST be named like the framework (no 'lib' prefix)
  cp "$dylib" "$outdir/${NAME}.framework/${NAME}"

  # If you didn’t set install_name via rustflags, fix it here:
  install_name_tool -id "@rpath/${NAME}.framework/${NAME}" "$outdir/${NAME}.framework/${NAME}"

  # Headers + module map
  cp "${INCLUDE_DIR}/ddg_url_predictor.h" "$outdir/${NAME}.framework/Headers/"
  cp "${INCLUDE_DIR}/module.modulemap" "$outdir/${NAME}.framework/Modules/"

  echo "${outdir}/${NAME}.framework"
}

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

# Build per-arch frameworks
F_MAC=$(make_framework "${DIST_DIR}/macos-arm64_x86_64" "macos")
F_IOS=$(make_framework "${DIST_DIR}/ios-arm64" "ios")
F_IOSSIM=$(make_framework "${DIST_DIR}/ios-arm64_x86_64-simulator" "iossim")

# Create the xcframework from the frameworks (not from raw dylibs)
rm -rf "${DIST_DIR}/${NAME}.xcframework"
xcodebuild -create-xcframework \
  -framework "$F_MAC" \
  -framework "$F_IOS" \
  -framework "$F_IOSSIM" \
  -output "${DIST_DIR}/${NAME}.xcframework"

echo "✅ Built ${DIST_DIR}/${NAME}.xcframework"

ditto -c -k --keepParent "${DIST_DIR}/${NAME}.xcframework" "${DIST_DIR}/${NAME}.xcframework.zip"

echo "✅ Zipped ${DIST_DIR}/${NAME}.xcframework.zip"

checksum="$(swift package compute-checksum "${DIST_DIR}/${NAME}.xcframework.zip")"
echo "Checksum: ${checksum}"
