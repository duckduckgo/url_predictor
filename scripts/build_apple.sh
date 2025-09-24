#!/usr/bin/env bash
set -euo pipefail

CRATE_NAME="url_predictor"
FEATURES="${FEATURES:-real-psl}"

NAME=URLPredictorRust                    # Framework & Swift module name
CRATE_LIB=liburl_predictor.dylib      # Rust cdylib filename
MIN_MACOS=11.3
MIN_IOS=15.0

DIST_DIR="dist/apple"

# Versions and identifiers
# Read library version from Cargo.toml and allow overriding bundle identifier via env
VERSION="$(grep -m1 '^version\s*=\s*\"' Cargo.toml | sed -E 's/.*\"([^\"]+)\".*/\1/')"
BUNDLE_ID="${BUNDLE_ID:-com.duckduckgo.URLPredictorRust}"

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

# Helper to write Info.plist into a framework bundle
write_info_plist () {
  local framework_dir="$1"   # e.g., dist/apple/ios-arm64/URLPredictorRust.framework
  local platform="$2"        # macos | ios | iossim
  local min_version
  if [[ "$platform" == "macos" ]]; then
    min_version="$MIN_MACOS"
  else
    min_version="$MIN_IOS"
  fi

  cat > "${framework_dir}/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>${NAME}</string>
  <key>CFBundleIdentifier</key>
  <string>${BUNDLE_ID}</string>
  <key>CFBundleExecutable</key>
  <string>${NAME}</string>
  <key>CFBundleShortVersionString</key>
  <string>${VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${VERSION}</string>
  <key>CFBundlePackageType</key>
  <string>FMWK</string>
  <key>MinimumOSVersion</key>
  <string>${min_version}</string>
</dict>
</plist>
EOF
}

# Helper to write a minimal PrivacyInfo.xcprivacy manifest
write_privacy_manifest () {
  local framework_dir="$1"
  cat > "${framework_dir}/PrivacyInfo.xcprivacy" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>NSPrivacyTracking</key>
  <false/>
  <key>NSPrivacyTrackingDomains</key>
  <array/>
  <key>NSPrivacyCollectedDataTypes</key>
  <array/>
  <key>NSPrivacyAccessedAPITypes</key>
  <array/>
</dict>
</plist>
EOF
}

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

  # macOS: build a versioned framework bundle
  if [[ "$platform" == "macos" ]]; then
    local framework_dir="${outdir}/${NAME}.framework"
    local versions_dir="${framework_dir}/Versions"
    local current_ver_dir="${versions_dir}/A"

    mkdir -p "${current_ver_dir}/Headers" "${current_ver_dir}/Modules" "${current_ver_dir}/Resources"

    # Binary
    cp "$dylib" "${current_ver_dir}/${NAME}"
    # install_name to versioned path
    install_name_tool -id "@rpath/${NAME}.framework/Versions/A/${NAME}" "${current_ver_dir}/${NAME}"

    # Headers + module map
    cp "${INCLUDE_DIR}/ddg_url_predictor.h" "${current_ver_dir}/Headers/"
    cp "${INCLUDE_DIR}/module.modulemap" "${current_ver_dir}/Modules/"

    # Metadata in Resources
    write_info_plist "${current_ver_dir}/Resources" "$platform"
    write_privacy_manifest "${current_ver_dir}/Resources"

    # Symlinks
    mkdir -p "${framework_dir}"
    ln -sfn "A" "${versions_dir}/Current"
    ln -sfn "Versions/Current/${NAME}" "${framework_dir}/${NAME}"
    ln -sfn "Versions/Current/Headers" "${framework_dir}/Headers"
    ln -sfn "Versions/Current/Modules" "${framework_dir}/Modules"
    ln -sfn "Versions/Current/Resources" "${framework_dir}/Resources"

    echo "${framework_dir}"
  else
    # iOS and iOS Simulator: shallow bundle
    mkdir -p "${outdir}/${NAME}.framework/Headers"
    mkdir -p "${outdir}/${NAME}.framework/Modules"

    # Binary inside framework MUST be named like the framework (no 'lib' prefix)
    cp "$dylib" "$outdir/${NAME}.framework/${NAME}"
    install_name_tool -id "@rpath/${NAME}.framework/${NAME}" "$outdir/${NAME}.framework/${NAME}"

    # Headers + module map
    cp "${INCLUDE_DIR}/ddg_url_predictor.h" "$outdir/${NAME}.framework/Headers/"
    cp "${INCLUDE_DIR}/module.modulemap" "$outdir/${NAME}.framework/Modules/"

    # Metadata at top-level for shallow bundles
    write_info_plist "$outdir/${NAME}.framework" "$platform"
    write_privacy_manifest "$outdir/${NAME}.framework"

    echo "${outdir}/${NAME}.framework"
  fi
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

# Generate dSYMs for each framework binary
MAC_BIN_PATH="${F_MAC}/Versions/Current/${NAME}"
IOS_BIN_PATH="${F_IOS}/${NAME}"
IOSSIM_BIN_PATH="${F_IOSSIM}/${NAME}"

MAC_DSYM_DIR="$(cd "$(dirname "${F_MAC}")" && pwd)/${NAME}.framework.dSYM"
IOS_DSYM_DIR="$(cd "$(dirname "${F_IOS}")" && pwd)/${NAME}.framework.dSYM"
IOSSIM_DSYM_DIR="$(cd "$(dirname "${F_IOSSIM}")" && pwd)/${NAME}.framework.dSYM"

dsymutil "${MAC_BIN_PATH}" -o "${MAC_DSYM_DIR}" >/dev/null
dsymutil "${IOS_BIN_PATH}" -o "${IOS_DSYM_DIR}" >/dev/null
dsymutil "${IOSSIM_BIN_PATH}" -o "${IOSSIM_DSYM_DIR}" >/dev/null

# Create the xcframework from the frameworks (not from raw dylibs), attaching dSYMs
rm -rf "${DIST_DIR}/${NAME}.xcframework"
xcodebuild -create-xcframework \
  -framework "$F_MAC" -debug-symbols "${MAC_DSYM_DIR}" \
  -framework "$F_IOS" -debug-symbols "${IOS_DSYM_DIR}" \
  -framework "$F_IOSSIM" -debug-symbols "${IOSSIM_DSYM_DIR}" \
  -output "${DIST_DIR}/${NAME}.xcframework"

echo "✅ Built ${DIST_DIR}/${NAME}.xcframework"

ditto -c -k --keepParent "${DIST_DIR}/${NAME}.xcframework" "${DIST_DIR}/${NAME}.xcframework.zip"

echo "✅ Zipped ${DIST_DIR}/${NAME}.xcframework.zip"

checksum="$(swift package compute-checksum "${DIST_DIR}/${NAME}.xcframework.zip")"
echo "Checksum: ${checksum}"
