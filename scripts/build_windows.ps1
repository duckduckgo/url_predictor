$ErrorActionPreference = "Stop"

$crate = "url_predictor"
$features = ${env:FEATURES}
if (-not $features) { $features = "real-psl" }

# Define targets
$targets = @(
    @{ name = "i686-pc-windows-msvc"; arch = "x86" },
    @{ name = "x86_64-pc-windows-msvc"; arch = "x64" },
    @{ name = "aarch64-pc-windows-msvc"; arch = "arm64" }
)

# Ensure targets
Write-Host "Installing targets..."
foreach ($target in $targets) {
    rustup target add $target.name
}

# Build all targets
Write-Host "Building all architectures..."
foreach ($target in $targets) {
    Write-Host "Building $($target.arch) version..."
    cargo build --release --features $features --target $target.name
}

# Layout - Clean and create directories
Remove-Item -Recurse -Force dist\windows -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force dist\windows\bin | Out-Null
New-Item -ItemType Directory -Force dist\windows\lib | Out-Null
New-Item -ItemType Directory -Force dist\include | Out-Null

# Copy artifacts for each architecture with arch suffix
foreach ($target in $targets) {
    $targetPath = "target\$($target.name)\release"
    $arch = $target.arch

    Write-Host "Copying $arch artifacts..."
    Copy-Item $targetPath\$crate.dll dist\windows\bin\$crate-$arch.dll
    Copy-Item $targetPath\$crate.lib dist\windows\lib\$crate-$arch.lib
    if (Test-Path $targetPath\$crate.pdb) {
        Copy-Item $targetPath\$crate.pdb dist\windows\bin\$crate-$arch.pdb
    }
}

# Header (architecture-independent)
if (-not (Get-Command cbindgen -ErrorAction SilentlyContinue)) {
    cargo install cbindgen
}
cbindgen --config cbindgen.toml --crate $crate --output dist\include\ddg_url_predictor.h

Write-Host "✅ Windows artifacts built for all architectures:"
Write-Host "   - DLLs: dist\windows\bin\ ($crate-x86.dll, $crate-x64.dll, $crate-arm64.dll)"
Write-Host "   - LIBs: dist\windows\lib\ ($crate-x86.lib, $crate-x64.lib, $crate-arm64.lib)"
Write-Host "   - Header: dist\include\ddg_url_predictor.h"
