# Tor v3 Vanity Address Generator

A GPU-accelerated vanity address generator for Tor v3 (.onion) hidden services with both CLI and GUI interfaces.

## Features

- **GPU Acceleration**: Uses NVIDIA CUDA for fast Ed25519 key generation
- **GUI Interface**: Simple egui-based graphical interface
- **Multi-Pattern Support**: Search for multiple patterns simultaneously
- **Position Matching**: Match patterns at prefix, suffix, or anywhere in address
- **Cross-Platform**: Works on Windows and Linux

## Screenshots

```
┌─────────────────────────────────────────────────┐
│  Tor v3 Vanity Address Generator                │
├─────────────────────────────────────────────────┤
│  Patterns (valid: a-z, 2-7):                    │
│  [mysite______] [Prefix ▼] [✕]                  │
│  [secret______] [Anywhere ▼] [✕]                │
│  [+ Add Pattern]                                │
│                                                 │
│  Output directory: [C:\Users\...\vanity_keys]   │
│  Estimated time: ~30 minutes                    │
│                                                 │
│  Status: Running on GeForce RTX 3080            │
│  Keys tried: 1,234,567,890                      │
│  Hash rate: 52,000,000/sec                      │
│                                                 │
│  [▶ Start]  [■ Stop]                            │
│                                                 │
│  Found addresses:                               │
│  12:34:56 mysiteabc123...onion [Copy]           │
└─────────────────────────────────────────────────┘
```

## Performance

On a GTX 1070 Ti:

| Prefix Length | Estimated Time |
|---------------|----------------|
| 5 characters  | ~7 minutes     |
| 6 characters  | ~3.5 hours     |
| 7 characters  | ~5 days        |
| 8 characters  | ~22 weeks      |

Note: Suffix and "anywhere" matching is slower than prefix matching.

## Installation

### Prerequisites

- **NVIDIA GPU** with CUDA support (GTX 900 series or newer)
- **NVIDIA Driver** 450.x or newer
- **CUDA Toolkit** 11.x or 12.x

### Windows

1. Install [CUDA Toolkit](https://developer.nvidia.com/cuda-downloads)
2. Install [Rust](https://rustup.rs) (choose MSVC toolchain)
3. Open PowerShell and run:

```powershell
rustup install nightly
rustup target add nvptx64-nvidia-cuda
cargo install ptx-linker

git clone https://github.com/Mxrider420z/tor-v3-vanity
cd tor-v3-vanity
cargo +nightly build --release
```

4. Find binaries in `target\release\`:
   - `t3v.exe` - CLI version
   - `t3v-gui.exe` - GUI version

### Linux

1. Install CUDA Toolkit:
```bash
# Ubuntu/Debian
sudo apt install nvidia-cuda-toolkit

# Or download from NVIDIA
# https://developer.nvidia.com/cuda-downloads
```

2. Install Rust and build:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup install nightly
rustup target add nvptx64-nvidia-cuda
cargo install ptx-linker

git clone https://github.com/Mxrider420z/tor-v3-vanity
cd tor-v3-vanity
cargo +nightly build --release
```

3. Find binaries in `target/release/`:
   - `t3v` - CLI version
   - `t3v-gui` - GUI version

### Pre-built Binaries

Download from [Releases](https://github.com/Mxrider420z/tor-v3-vanity/releases).

## Usage

### GUI

Simply run `t3v-gui` (or `t3v-gui.exe` on Windows):

1. Enter one or more patterns (valid characters: a-z, 2-7)
2. Select position for each pattern (Prefix, Suffix, or Anywhere)
3. Choose output directory
4. Click Start

Found keys are saved as Tor-compatible `hs_ed25519_secret_key` files.

### CLI

```bash
# Basic usage - find addresses starting with "mysite"
t3v --dst ./keys mysite

# Multiple patterns
t3v --dst ./keys mysite,secret,hidden

# Show help
t3v --help
```

### Using Generated Keys

Copy the generated file to your Tor hidden service directory:

```bash
# Linux
cp myprefixabc123...onion /var/lib/tor/hidden_service/hs_ed25519_secret_key
sudo chown debian-tor:debian-tor /var/lib/tor/hidden_service/hs_ed25519_secret_key
sudo chmod 600 /var/lib/tor/hidden_service/hs_ed25519_secret_key
sudo systemctl restart tor

# Windows (in Tor Browser Bundle)
copy myprefixabc123...onion "C:\path\to\tor\hidden_service\hs_ed25519_secret_key"
```

## Building CLI Only

To build without GUI dependencies:

```bash
cargo +nightly build --release --no-default-features
```

## Troubleshooting

### "No CUDA devices available"
- Ensure you have an NVIDIA GPU
- Install/update NVIDIA drivers
- Install CUDA Toolkit

### Build fails with "ptx-linker not found"
```bash
cargo install ptx-linker
```

### Build fails with "nvptx64 target not found"
```bash
rustup target add nvptx64-nvidia-cuda
```

### Windows: "CUDA_PATH not set"
- Reinstall CUDA Toolkit
- Or set manually: `$env:CUDA_PATH = "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.0"`

## Security Notes

- Keys are generated using cryptographically secure random number generation
- Private keys are written directly to disk and not logged
- The GPU kernel uses ed25519-compact for key generation

## License

MIT License - see [LICENSE](LICENSE)

## Credits

Based on [tor-v3-vanity](https://github.com/dr-bonez/tor-v3-vanity) by Aiden McClelland.
GUI and Windows support added by Mxrider420z.
