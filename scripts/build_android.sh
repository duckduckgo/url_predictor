#!/usr/bin/env bash
set -euo pipefail

# --- Config (override via env or flags) ---
LIB_NAME="${LIB_NAME:-url_predictor}"                          # Rust crate/library name (produces lib${LIB_NAME}.so)
ANDROID_MODULE="${ANDROID_MODULE:-android/ddg-url-predictor}"  # Android library module path
BUILD_TYPE="${BUILD_TYPE:-release}"                            # release|debug
FEATURES="${FEATURES:-real-psl}"                               # cargo features (empty for none)
ABIS="${ABIS:-arm64-v8a armeabi-v7a x86_64 x86}"              # ABIs to build
PUBLISH="${PUBLISH:-false}"                                    # Whether to publish to Maven (default: false)

# --- Flags ---
#   --debug / --release
#   --features "feat1,feat2" or "" to disable
#   --module path/to/module
#   --lib-name my_lib
#   --abis "arm64-v8a x86_64"
#   --publish (to publish to Maven)
while [[ $# -gt 0 ]]; do
  case "$1" in
    --debug) BUILD_TYPE="debug"; shift;;
    --release) BUILD_TYPE="release"; shift;;
    --features) FEATURES="${2-}"; shift 2;;
    --module) ANDROID_MODULE="${2-}"; shift 2;;
    --lib-name) LIB_NAME="${2-}"; shift 2;;
    --abis) ABIS="${2-}"; shift 2;;
    --publish) PUBLISH="true"; shift;;
    -h|--help)
      cat <<EOF
Usage: $0 [options]

Options:
  --debug | --release           Build type (default: ${BUILD_TYPE})
  --features "feat1,feat2"      Cargo features (default: "${FEATURES}")
  --module <path>               Android module path (default: ${ANDROID_MODULE})
  --lib-name <name>             Rust library name (default: ${LIB_NAME})
  --abis "a b c"                ABIs (default: ${ABIS})
  --publish                     Publish the built AAR to Maven (default: false)
EOF
      exit 0;;
    *)
      echo "Unknown arg: $1" >&2; exit 1;;
  esac
done

# --- Ensure cargo-ndk is available ---
if ! command -v cargo-ndk >/dev/null 2>&1; then
  echo "Installing cargo-ndk..."
  cargo install cargo-ndk
fi

# --- Resolve output dir for jniLibs ---
JNI_LIBS_DIR="${ANDROID_MODULE}/src/main/jniLibs"
echo "JNI libs dir: ${JNI_LIBS_DIR}"
rm -rf "${JNI_LIBS_DIR}"
mkdir -p "${JNI_LIBS_DIR}"

# --- Build flags ---
CARGO_FLAGS=()
if [[ "${BUILD_TYPE}" == "release" ]]; then
  CARGO_FLAGS+=(build --release)
else
  CARGO_FLAGS+=(build)
fi

FEATURE_FLAGS=()
if [[ -n "${FEATURES}" ]]; then
  # cargo expects comma-separated features for -F/--features
  FEATURE_FLAGS+=(--features "${FEATURES}")
fi

# Ensure ELF LOAD segments are aligned to at least 16KB (2**14).
export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=0x4000 -Wl,-z,common-page-size=16384"

# --- Build per ABI ---
for ABI in ${ABIS}; do
  echo "==> Building for ABI: ${ABI}"
  cargo ndk -t "${ABI}" -o "${JNI_LIBS_DIR}" "${CARGO_FLAGS[@]}" "${FEATURE_FLAGS[@]}"
done

# Verify artifacts exist
echo "Built libraries:"
find "${JNI_LIBS_DIR}" -type f -name "lib${LIB_NAME}.so" -print

# --- Build Android AAR ---
echo "==> Building Android AAR..."
pushd "$(dirname "${ANDROID_MODULE}")" >/dev/null
./gradlew clean ":$(basename "${ANDROID_MODULE}")":assembleRelease

# --- Optionally publish to Maven ---
if [[ "${PUBLISH}" == "true" ]]; then
  echo "==> Publishing AAR to Maven..."
  ./gradlew --no-daemon --stacktrace --info --warning-mode all ":$(basename "${ANDROID_MODULE}")":publish
else
  echo "Skipping Maven publish (use --publish to enable)"
fi

popd >/dev/null

echo "Done."
echo "AAR should be at: ${ANDROID_MODULE}/build/outputs/aar/$(basename "${ANDROID_MODULE}")-release.aar"
