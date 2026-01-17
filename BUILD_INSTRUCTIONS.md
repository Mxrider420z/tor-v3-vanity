# Tor V3 Vanity Generator - Build Instructions

## Project Structure

```
tor-v3-vanity/
├── src/
│   ├── lib.rs           # Library exports
│   ├── main.rs          # CLI application
│   ├── onion.rs         # Onion address generation
│   └── backend/
│       ├── mod.rs       # Backend trait & selection
│       ├── cpu.rs       # CPU backend (Rayon)
│       ├── cuda.rs      # CUDA GPU backend
│       └── hybrid.rs    # Hybrid CPU+GPU backend
├── gui/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs      # GUI entry point
│       └── app.rs       # egui application
├── core/                # CUDA kernel (unchanged)
├── Cargo.toml           # Workspace root
├── build.rs             # Cross-platform CUDA detection
└── BUILD_INSTRUCTIONS.md
```

## Features

- **3 Backend Modes:**
  - CPU only (Rayon parallel processing)
  - GPU only (CUDA)
  - Hybrid (CPU + GPU combined for maximum speed)

- **Configurable CPU threads**
- **Cross-platform** (Linux, Windows)
- **GUI** (egui/eframe)
- **Portable .exe** on Windows

---

## Prerequisites

### Linux

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# For CUDA support (optional):
# Install NVIDIA CUDA Toolkit from https://developer.nvidia.com/cuda-downloads
# Or via package manager:
sudo apt install nvidia-cuda-toolkit
```

### Windows

1. **Visual Studio Build Tools 2022**
   - Download from https://visualstudio.microsoft.com/downloads/
   - Install "Desktop development with C++"

2. **Rust**
   ```powershell
   winget install Rustlang.Rustup
   # Or download from https://rustup.rs/
   ```

3. **NVIDIA CUDA Toolkit** (optional, for GPU support)
   - Download from https://developer.nvidia.com/cuda-downloads
   - Ensure CUDA_PATH environment variable is set

---

## Build Commands

### CPU-Only Build (Portable, No CUDA Required)

```bash
# CLI
cargo build --release --no-default-features

# GUI
cargo build --release -p t3v-gui --no-default-features
```

Output:
- `target/release/t3v` (or `t3v.exe` on Windows)
- `target/release/t3v-gui` (or `t3v-gui.exe` on Windows)

### Full Build with CUDA

Requires CUDA Toolkit and Rust nightly:

```bash
# Install nightly and CUDA target
rustup toolchain install nightly
rustup target add nvptx64-nvidia-cuda
cargo install ptx-linker

# Build CLI with CUDA
cargo +nightly build --release

# Build GUI with CUDA
cargo +nightly build --release -p t3v-gui
```

---

## Usage

### CLI

```bash
# Basic usage (auto-select best backend)
t3v myprefix,another -d ./output

# Force CPU mode with 8 threads
t3v --mode cpu --threads 8 myprefix -d ./output

# Force GPU mode
t3v --mode cuda myprefix -d ./output

# Hybrid mode (CPU + GPU)
t3v --mode hybrid myprefix -d ./output

# See all options
t3v --help
```

### GUI

Launch `t3v-gui` and:
1. Select backend mode (Auto/CPU/GPU/Hybrid)
2. Adjust CPU threads if needed
3. Enter prefixes (comma-separated)
4. Select output directory
5. Click "Start Generation"

---

## Performance Estimates

| Backend | Hardware | Est. Speed |
|---------|----------|------------|
| CPU | i7-12700 (20 threads) | ~8-10M keys/sec |
| CPU | Ryzen 9 5900X (24 threads) | ~10-12M keys/sec |
| CUDA | GTX 1070 Ti | ~150M keys/sec |
| CUDA | RTX 3080 | ~400M keys/sec |
| Hybrid | RTX 3080 + i7-12700 | ~410M keys/sec |

Time to find a prefix (average):
- 4 chars: ~1 second
- 5 chars: ~30 seconds
- 6 chars: ~15 minutes
- 7 chars: ~8 hours

---

## Troubleshooting

### CUDA not detected

1. Ensure NVIDIA drivers are installed
2. Ensure CUDA Toolkit is installed
3. Check that `CUDA_PATH` environment variable is set
4. Try running with `--mode cpu` to use CPU-only mode

### Build errors on Windows

1. Ensure Visual Studio Build Tools are installed
2. Run from "Developer Command Prompt for VS"
3. For CUDA builds, use the nightly toolchain

### PTX linker errors

```bash
cargo install ptx-linker --force
rustup target add nvptx64-nvidia-cuda
```

---

## License

MIT License
