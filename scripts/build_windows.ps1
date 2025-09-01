$ErrorActionPreference = "Stop"

$crate = "url_predictor"
$features = ${env:FEATURES}
if (-not $features) { $features = "real-psl" }

# Ensure target
rustup target add x86_64-pc-windows-msvc

# Build
cargo build --release --features $features --target x86_64-pc-windows-msvc

# Layout
Remove-Item -Recurse -Force dist\windows -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force dist\windows\bin | Out-Null
New-Item -ItemType Directory -Force dist\windows\lib | Out-Null
New-Item -ItemType Directory -Force dist\include | Out-Null

Copy-Item target\x86_64-pc-windows-msvc\release\$crate.dll dist\windows\bin\
Copy-Item target\x86_64-pc-windows-msvc\release\$crate.lib dist\windows\lib\
if (Test-Path target\x86_64-pc-windows-msvc\release\$crate.pdb) {
  Copy-Item target\x86_64-pc-windows-msvc\release\$crate.pdb dist\windows\bin\
}

# Header
if (-not (Get-Command cbindgen -ErrorAction SilentlyContinue)) {
  cargo install cbindgen
}
cbindgen --config cbindgen.toml --crate $crate --output dist\include\ddg_url_predictor.h

Write-Host "✅ Windows artifacts at dist\windows\ (dll/lib/pdb) and header in dist\include"

