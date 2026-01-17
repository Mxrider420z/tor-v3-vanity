use std::path::Path;

fn main() {
    // Only run CUDA build steps if the cuda feature is enabled
    #[cfg(feature = "cuda")]
    {
        cuda_build();
    }
}

#[cfg(feature = "cuda")]
fn cuda_build() {
    use std::env;
    use std::path::PathBuf;

    // Platform-specific CUDA library paths
    #[cfg(target_os = "windows")]
    {
        // Try to find CUDA on Windows
        if let Ok(cuda_path) = env::var("CUDA_PATH") {
            let lib_path = Path::new(&cuda_path).join("lib").join("x64");
            if lib_path.exists() {
                println!("cargo:rustc-link-search=native={}", lib_path.display());
            }
        } else {
            // Try common installation paths
            let cuda_paths = [
                r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6\lib\x64",
                r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.5\lib\x64",
                r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.4\lib\x64",
                r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.3\lib\x64",
                r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.2\lib\x64",
                r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.1\lib\x64",
                r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.0\lib\x64",
                r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v11.8\lib\x64",
                r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v11.7\lib\x64",
            ];

            for path in &cuda_paths {
                if Path::new(path).exists() {
                    println!("cargo:rustc-link-search=native={}", path);
                    break;
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try standard Linux CUDA paths
        let cuda_paths = [
            "/usr/local/cuda/lib64",
            "/usr/lib/x86_64-linux-gnu",
            "/opt/cuda/lib64",
        ];

        for path in &cuda_paths {
            if Path::new(path).exists() {
                println!("cargo:rustc-link-search=native={}", path);
                break;
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        // CUDA on macOS (legacy, NVIDIA no longer supports)
        if let Ok(cuda_path) = env::var("CUDA_PATH") {
            let lib_path = Path::new(&cuda_path).join("lib");
            if lib_path.exists() {
                println!("cargo:rustc-link-search=native={}", lib_path.display());
            }
        }
    }

    // Use pre-compiled PTX kernel instead of building with ptx-builder
    // This avoids the need for ptx-linker which has LLVM version compatibility issues
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let ptx_path = PathBuf::from(&manifest_dir).join("ptx").join("kernel.ptx");

    if ptx_path.exists() {
        // Set the environment variable that the CUDA backend expects
        println!("cargo:rustc-env=KERNEL_PTX_PATH={}", ptx_path.display());
        println!("cargo:rerun-if-changed={}", ptx_path.display());
        println!("cargo:warning=Using pre-compiled PTX kernel from {}", ptx_path.display());
    } else {
        // Fall back to building with ptx-builder if available
        println!("cargo:warning=Pre-compiled PTX not found at {}, attempting to build...", ptx_path.display());

        #[cfg(feature = "ptx-builder")]
        {
            use ptx_builder::error::Result;
            use ptx_builder::prelude::*;

            fn build_ptx() -> Result<()> {
                std::env::set_var("CARGO_BUILD_PIPELINING", "false");
                let builder = Builder::new("core")?;
                CargoAdapter::with_env_var("KERNEL_PTX_PATH").build(builder);
                Ok(())
            }

            if let Err(e) = build_ptx() {
                panic!("PTX kernel not found and build failed: {}. Please ensure ptx/kernel.ptx exists.", e);
            }
        }

        #[cfg(not(feature = "ptx-builder"))]
        {
            panic!(
                "Pre-compiled PTX kernel not found at {}.\n\
                Please copy the kernel.ptx file to the ptx/ directory.\n\
                You can compile it on Linux with: cargo build -p tor-v3-vanity-core --target nvptx64-nvidia-cuda --release",
                ptx_path.display()
            );
        }
    }
}
