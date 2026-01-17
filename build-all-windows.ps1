# Tor V3 Vanity Generator - Complete Windows Build Script
# Builds both the Rust GUI and the CUDA generator
#
# Prerequisites:
#   - Rust toolchain (https://rustup.rs/)
#   - CUDA Toolkit 12.x (https://developer.nvidia.com/cuda-downloads)
#   - CMake (https://cmake.org/download/)
#   - Visual Studio Build Tools with C++ workload

param(
    [switch]$SkipCuda,
    [switch]$SkipGui,
    [switch]$Help
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

if ($Help) {
    Write-Host @"
Tor V3 Vanity Generator - Complete Windows Build

Usage: .\build-all-windows.ps1 [options]

Options:
    -SkipCuda   Skip building CUDA generator (GUI only)
    -SkipGui    Skip building GUI (CUDA only)
    -Help       Show this help

This script builds:
1. vanity_torv3_cuda.exe - GPU-accelerated generator (requires CUDA + CMake)
2. t3v-gui.exe - GUI application (requires Rust)

Prerequisites:
    - Rust: https://rustup.rs/
    - CUDA Toolkit: https://developer.nvidia.com/cuda-downloads
    - CMake: https://cmake.org/download/
    - Visual Studio Build Tools with C++ workload

"@
    exit 0
}

Write-Host ""
Write-Host "=== Tor V3 Vanity Generator - Complete Build ===" -ForegroundColor Cyan
Write-Host ""

# Create output directory
$distDir = Join-Path $ScriptDir "dist"
if (-not (Test-Path $distDir)) {
    New-Item -ItemType Directory -Path $distDir | Out-Null
}

# ============================================
# STEP 1: Build CUDA Generator
# ============================================
if (-not $SkipCuda) {
    Write-Host "=== Step 1: Building CUDA Generator ===" -ForegroundColor Yellow
    Write-Host ""

    # Check for CMake
    $cmakePath = Get-Command cmake -ErrorAction SilentlyContinue
    if (-not $cmakePath) {
        Write-Host "[WARN] CMake not found. Skipping CUDA build." -ForegroundColor Yellow
        Write-Host "       Install from: https://cmake.org/download/" -ForegroundColor Gray
        $SkipCuda = $true
    }

    # Check for CUDA
    if (-not $SkipCuda) {
        $cudaPath = $env:CUDA_PATH
        if (-not $cudaPath) {
            $defaultPaths = @(
                "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6",
                "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.5",
                "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.4",
                "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.3",
                "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.2"
            )
            foreach ($path in $defaultPaths) {
                if (Test-Path $path) {
                    $cudaPath = $path
                    $env:CUDA_PATH = $path
                    break
                }
            }
        }

        if (-not $cudaPath) {
            Write-Host "[WARN] CUDA not found. Skipping CUDA build." -ForegroundColor Yellow
            Write-Host "       Install from: https://developer.nvidia.com/cuda-downloads" -ForegroundColor Gray
            $SkipCuda = $true
        } else {
            Write-Host "[OK] CUDA found at: $cudaPath" -ForegroundColor Green
        }
    }

    if (-not $SkipCuda) {
        # Download CUDA project if not present
        $cudaProjectDir = Join-Path $ScriptDir "torv3_vanity_addr_cuda"
        if (-not (Test-Path $cudaProjectDir)) {
            Write-Host "[INFO] Downloading CUDA project..." -ForegroundColor White

            # Try git first, fall back to zip download
            $gitPath = Get-Command git -ErrorAction SilentlyContinue
            $downloaded = $false

            if ($gitPath) {
                git clone https://github.com/Danukeru/torv3_vanity_addr_cuda.git $cudaProjectDir 2>$null
                if ($LASTEXITCODE -eq 0) {
                    $downloaded = $true
                    Write-Host "[OK] Cloned via git" -ForegroundColor Green
                }
            }

            if (-not $downloaded) {
                # Download as zip (no git required)
                $zipUrl = "https://github.com/Danukeru/torv3_vanity_addr_cuda/archive/refs/heads/master.zip"
                $zipFile = Join-Path $ScriptDir "cuda_project.zip"

                try {
                    Write-Host "[INFO] Downloading from GitHub (no git required)..." -ForegroundColor White
                    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
                    Invoke-WebRequest -Uri $zipUrl -OutFile $zipFile -UseBasicParsing

                    Write-Host "[INFO] Extracting..." -ForegroundColor White
                    Expand-Archive -Path $zipFile -DestinationPath $ScriptDir -Force

                    # Rename extracted folder (GitHub adds -master suffix)
                    $extractedDir = Join-Path $ScriptDir "torv3_vanity_addr_cuda-master"
                    if (Test-Path $extractedDir) {
                        Rename-Item $extractedDir $cudaProjectDir
                    }

                    Remove-Item $zipFile -Force
                    $downloaded = $true
                    Write-Host "[OK] Downloaded and extracted" -ForegroundColor Green
                } catch {
                    Write-Host "[ERROR] Failed to download CUDA project: $_" -ForegroundColor Red
                }
            }

            if (-not $downloaded) {
                Write-Host "[ERROR] Failed to get CUDA project" -ForegroundColor Red
                $SkipCuda = $true
            }
        }

        if (-not $SkipCuda) {
            # Build CUDA project
            $cudaBuildDir = Join-Path $cudaProjectDir "build"
            if (-not (Test-Path $cudaBuildDir)) {
                New-Item -ItemType Directory -Path $cudaBuildDir | Out-Null
            }

            Push-Location $cudaBuildDir
            try {
                Write-Host "[INFO] Configuring CUDA project..." -ForegroundColor White

                # Temporarily allow errors (CMake warnings go to stderr)
                $oldErrorAction = $ErrorActionPreference
                $ErrorActionPreference = "Continue"

                # Try different Visual Studio generators
                $configured = $false
                $generators = @(
                    "Visual Studio 17 2022",
                    "Visual Studio 16 2019",
                    "Visual Studio 15 2017"
                )

                foreach ($gen in $generators) {
                    Write-Host "[INFO] Trying $gen..." -ForegroundColor Gray
                    $output = & cmake .. -G $gen -A x64 2>&1
                    if ($LASTEXITCODE -eq 0) {
                        $configured = $true
                        Write-Host "[OK] Configured with $gen" -ForegroundColor Green
                        break
                    }
                }

                if (-not $configured) {
                    # Try Ninja if VS fails
                    Write-Host "[INFO] Trying Ninja..." -ForegroundColor Gray
                    $output = & cmake .. -G Ninja 2>&1
                    if ($LASTEXITCODE -eq 0) {
                        $configured = $true
                        Write-Host "[OK] Configured with Ninja" -ForegroundColor Green
                    }
                }

                $ErrorActionPreference = $oldErrorAction

                if ($configured) {
                    Write-Host "[INFO] Building CUDA project..." -ForegroundColor White
                    cmake --build . --config Release

                    if ($LASTEXITCODE -eq 0) {
                        # Find and copy the executable
                        $cudaExe = Get-ChildItem -Path . -Recurse -Filter "vanity_torv3_cuda.exe" | Select-Object -First 1
                        if ($cudaExe) {
                            Copy-Item $cudaExe.FullName (Join-Path $distDir "vanity_torv3_cuda.exe") -Force
                            Write-Host "[OK] CUDA generator built successfully" -ForegroundColor Green
                        } else {
                            Write-Host "[WARN] CUDA executable not found after build" -ForegroundColor Yellow
                        }
                    } else {
                        Write-Host "[ERROR] CUDA build failed" -ForegroundColor Red
                    }
                } else {
                    Write-Host "[ERROR] CMake configuration failed. Output:" -ForegroundColor Red
                    Write-Host $output -ForegroundColor Gray
                }
            } finally {
                Pop-Location
            }
        }
    }
    Write-Host ""
}

# ============================================
# STEP 2: Build Rust GUI
# ============================================
if (-not $SkipGui) {
    Write-Host "=== Step 2: Building Rust GUI ===" -ForegroundColor Yellow
    Write-Host ""

    # Check for Rust
    $rustPath = Get-Command rustc -ErrorAction SilentlyContinue
    if (-not $rustPath) {
        Write-Host "[ERROR] Rust not found. Install from: https://rustup.rs/" -ForegroundColor Red
        exit 1
    }
    Write-Host "[OK] Rust found: $(rustc --version)" -ForegroundColor Green

    # Build GUI (without cuda feature - uses external CUDA)
    Write-Host "[INFO] Building GUI..." -ForegroundColor White
    Push-Location $ScriptDir
    try {
        cargo build -p t3v-gui --release
        if ($LASTEXITCODE -eq 0) {
            Copy-Item "target\release\t3v-gui.exe" (Join-Path $distDir "t3v-gui.exe") -Force
            Write-Host "[OK] GUI built successfully" -ForegroundColor Green
        } else {
            Write-Host "[ERROR] GUI build failed" -ForegroundColor Red
            exit 1
        }

        # Also build CLI
        Write-Host "[INFO] Building CLI..." -ForegroundColor White
        cargo build -p tor-v3-vanity --bin t3v --release
        if ($LASTEXITCODE -eq 0) {
            Copy-Item "target\release\t3v.exe" (Join-Path $distDir "t3v.exe") -Force
            Write-Host "[OK] CLI built successfully" -ForegroundColor Green
        }
    } finally {
        Pop-Location
    }
    Write-Host ""
}

# ============================================
# Summary
# ============================================
Write-Host "=== Build Complete! ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "Output directory: $distDir" -ForegroundColor White
Write-Host ""

$files = Get-ChildItem $distDir -Filter "*.exe"
foreach ($file in $files) {
    $size = [math]::Round($file.Length / 1MB, 2)
    Write-Host "  $($file.Name) ($size MB)" -ForegroundColor Green
}

Write-Host ""
Write-Host "Usage:" -ForegroundColor White
Write-Host "  1. Run t3v-gui.exe" -ForegroundColor Gray
Write-Host "  2. Select GPU mode (requires vanity_torv3_cuda.exe in same folder)" -ForegroundColor Gray
Write-Host "  3. Enter prefix and click Start" -ForegroundColor Gray
Write-Host ""

if (-not (Test-Path (Join-Path $distDir "vanity_torv3_cuda.exe"))) {
    Write-Host "[NOTE] CUDA generator not built. GPU mode will fall back to CPU." -ForegroundColor Yellow
    Write-Host "       To enable GPU: Install CUDA + CMake, then run this script again." -ForegroundColor Gray
}
Write-Host ""
