# Tor V3 Vanity Address Generator - Windows Build Script
# Run this script in the tor-v3-vanity source folder to build everything
# Prerequisites: Rust toolchain, CUDA Toolkit (optional, for GPU support)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

Write-Host ""
Write-Host "=== Tor V3 Vanity Generator - Windows Build ===" -ForegroundColor Cyan
Write-Host ""

# Check for Rust
Write-Host "[INFO] Checking Rust installation..." -ForegroundColor White
try {
    $rustVersion = rustc --version
    Write-Host "[OK] Rust found: $rustVersion" -ForegroundColor Green
} catch {
    Write-Host "[ERROR] Rust not found. Please install from https://rustup.rs/" -ForegroundColor Red
    Write-Host ""
    Write-Host "Quick install:" -ForegroundColor Yellow
    Write-Host "  1. Download from: https://win.rustup.rs/x86_64" -ForegroundColor Gray
    Write-Host "  2. Run the installer" -ForegroundColor Gray
    Write-Host "  3. Restart this terminal" -ForegroundColor Gray
    Write-Host ""
    exit 1
}

# Check for CUDA
$cudaAvailable = $false
$cudaPath = $null

Write-Host "[INFO] Checking for CUDA..." -ForegroundColor White

if ($env:CUDA_PATH -and (Test-Path $env:CUDA_PATH)) {
    $cudaPath = $env:CUDA_PATH
    Write-Host "[OK] CUDA found at: $cudaPath" -ForegroundColor Green
    $cudaAvailable = $true
} else {
    $defaultPaths = @(
        "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6",
        "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.5",
        "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.4",
        "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.3",
        "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.2",
        "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.1",
        "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.0",
        "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v11.8",
        "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v11.7"
    )

    foreach ($path in $defaultPaths) {
        if (Test-Path $path) {
            $cudaPath = $path
            $env:CUDA_PATH = $path
            Write-Host "[OK] CUDA found at: $path" -ForegroundColor Green
            $cudaAvailable = $true
            break
        }
    }
}

# Check for pre-compiled PTX kernel
$ptxPath = Join-Path $ScriptDir "ptx\kernel.ptx"
if ($cudaAvailable) {
    if (Test-Path $ptxPath) {
        Write-Host "[OK] Pre-compiled PTX kernel found" -ForegroundColor Green
    } else {
        Write-Host "[WARN] PTX kernel not found at: $ptxPath" -ForegroundColor Yellow
        Write-Host "[WARN] Building without CUDA support (CPU-only)" -ForegroundColor Yellow
        $cudaAvailable = $false
    }
}

if (-not $cudaAvailable) {
    Write-Host "[INFO] Building CPU-only version" -ForegroundColor White
    Write-Host "       (For GPU support, install CUDA Toolkit and copy ptx/kernel.ptx)" -ForegroundColor Gray
}

Write-Host ""
Write-Host "=== Building ===" -ForegroundColor Cyan
Write-Host ""

# Determine features
$features = if ($cudaAvailable) { "--features cuda" } else { "--no-default-features" }

# Build CLI
Write-Host "[INFO] Building CLI (t3v.exe)..." -ForegroundColor White
$cliCmd = "cargo build -p tor-v3-vanity --bin t3v --release $features"
Write-Host "       Running: $cliCmd" -ForegroundColor Gray
Invoke-Expression $cliCmd

if ($LASTEXITCODE -eq 0) {
    Write-Host "[OK] CLI built successfully" -ForegroundColor Green
} else {
    Write-Host "[ERROR] Failed to build CLI" -ForegroundColor Red
    exit 1
}

# Build GUI
Write-Host "[INFO] Building GUI (t3v-gui.exe)..." -ForegroundColor White
$guiFeatures = if ($cudaAvailable) { "--features cuda" } else { "" }
$guiCmd = "cargo build -p t3v-gui --release $guiFeatures"
Write-Host "       Running: $guiCmd" -ForegroundColor Gray
Invoke-Expression $guiCmd

if ($LASTEXITCODE -eq 0) {
    Write-Host "[OK] GUI built successfully" -ForegroundColor Green
} else {
    Write-Host "[ERROR] Failed to build GUI" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "=== Build Complete! ===" -ForegroundColor Cyan
Write-Host ""

# Copy to output folder
$outputDir = Join-Path $ScriptDir "dist"
if (-not (Test-Path $outputDir)) {
    New-Item -ItemType Directory -Path $outputDir | Out-Null
}

$targetDir = Join-Path $ScriptDir "target\release"
$cliSrc = Join-Path $targetDir "t3v.exe"
$guiSrc = Join-Path $targetDir "t3v-gui.exe"
$cliDst = Join-Path $outputDir "t3v.exe"
$guiDst = Join-Path $outputDir "t3v-gui.exe"

if (Test-Path $cliSrc) {
    Copy-Item $cliSrc $cliDst -Force
    $cliSize = [math]::Round((Get-Item $cliDst).Length / 1MB, 2)
    Write-Host "[OK] CLI: $cliDst ($cliSize MB)" -ForegroundColor Green
}

if (Test-Path $guiSrc) {
    Copy-Item $guiSrc $guiDst -Force
    $guiSize = [math]::Round((Get-Item $guiDst).Length / 1MB, 2)
    Write-Host "[OK] GUI: $guiDst ($guiSize MB)" -ForegroundColor Green
}

# Copy PTX to dist if building with CUDA
if ($cudaAvailable -and (Test-Path $ptxPath)) {
    $ptxDst = Join-Path $outputDir "kernel.ptx"
    Copy-Item $ptxPath $ptxDst -Force
    Write-Host "[OK] PTX: $ptxDst" -ForegroundColor Green
}

Write-Host ""
if ($cudaAvailable) {
    Write-Host "Built with CUDA GPU support!" -ForegroundColor Green
} else {
    Write-Host "Built CPU-only version" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "Output files are in: $outputDir" -ForegroundColor Cyan
Write-Host ""
Write-Host "Usage:" -ForegroundColor White
Write-Host "  CLI: .\dist\t3v.exe myprefix,cool --dst .\keys" -ForegroundColor Gray
Write-Host "  GUI: .\dist\t3v-gui.exe" -ForegroundColor Gray
Write-Host ""
