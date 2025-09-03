#!/usr/bin/env bash
set -euo pipefail

# ---- Config ----
crate="url_predictor"
FEATURES="${FEATURES:-real-psl}"
target_triple="x86_64-pc-windows-msvc"

echo "==> Using FEATURES='${FEATURES}' and target='${target_triple}'"

# ---- Ensure target ----
rustup target add "${target_triple}"

# ---- Build ----
cargo build --release --features "${FEATURES}" --target "${target_triple}"

# ---- Layout ----
rm -rf dist/windows || true
mkdir -p dist/windows/bin dist/windows/lib dist/include

# Copy artifacts
dll_path="target/${target_triple}/release/${crate}.dll"
lib_path="target/${target_triple}/release/${crate}.lib"
pdb_path="target/${target_triple}/release/${crate}.pdb"

cp "${dll_path}" dist/windows/bin/
cp "${lib_path}" dist/windows/lib/
if [ -f "${pdb_path}" ]; then
  cp "${pdb_path}" dist/windows/bin/
fi

# ---- Header via cbindgen ----
if ! command -v cbindgen >/dev/null 2>&1; then
  echo "==> cbindgen not found; installing with Cargo..."
  cargo install cbindgen
fi

cbindgen --config cbindgen.toml --crate "${crate}" --output "dist/include/ddg_url_predictor.h"

printf "\n✅ Windows artifacts at dist/windows (dll/lib/pdb) and header in dist/include\n"

