#!/usr/bin/env bash
set -euo pipefail

PSL_URL="https://publicsuffix.org/list/public_suffix_list.dat"
TARGET="assets/public_suffix_list.dat"

echo "Downloading latest Public Suffix List..."
curl -sSf "$PSL_URL" -o "$TARGET"

echo "Updated $TARGET"

