#!/usr/bin/env bash
set -euo pipefail

LIB_NAME="url_predictor"

# Resolve script dir (no matter where Gradle runs it from)
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Rust crate root (one level above scripts/)
RUST_ROOT="${SCRIPT_DIR}/.."         # adjust if necessary
RUST_TARGET_DIR="${RUST_ROOT}/target/release"

# Android module JVM test folder
ANDROID_MODULE="${RUST_ROOT}/android/ddg-url-predictor"
TEST_JNI="${ANDROID_MODULE}/src/test/resources/jni"

echo "SCRIPT_DIR        = ${SCRIPT_DIR}"
echo "RUST_ROOT         = ${RUST_ROOT}"
echo "RUST_TARGET_DIR   = ${RUST_TARGET_DIR}"
echo "TEST_JNI          = ${TEST_JNI}"

echo "==> Building Rust library for HOST..."
(
  cd "${RUST_ROOT}"
  cargo build --release --features "real-psl jni-host-tests"
)

mkdir -p "${TEST_JNI}"

# Mac or Linux detection
HOST_LIB=""
if [[ "$OSTYPE" == "darwin"* ]]; then
    HOST_LIB="${RUST_TARGET_DIR}/lib${LIB_NAME}.dylib"
elif [[ "$OSTYPE" == "linux"* ]]; then
    HOST_LIB="${RUST_TARGET_DIR}/lib${LIB_NAME}.so"
else
    echo "Unsupported OS: $OSTYPE"
    exit 1
fi

if [[ ! -f "$HOST_LIB" ]]; then
    echo "ERROR: Host library missing: $HOST_LIB"
    exit 1
fi

echo "Copying $HOST_LIB → $TEST_JNI/"
cp "$HOST_LIB" "$TEST_JNI"

echo "==> Done."

