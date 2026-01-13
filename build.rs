use ptx_builder::error::Result;
use ptx_builder::prelude::*;

fn main() -> Result<()> {
    // Workaround for "crate required to be available in rlib format" bug
    std::env::set_var("CARGO_BUILD_PIPELINING", "false");

    // Help cargo find libcuda - platform-aware paths
    #[cfg(target_os = "windows")]
    {
        // Windows: Use CUDA_PATH environment variable set by CUDA installer
        if let Ok(cuda_path) = std::env::var("CUDA_PATH") {
            println!("cargo:rustc-link-search=native={}\\lib\\x64", cuda_path);
        } else {
            // Fallback to common Windows CUDA installation paths
            println!("cargo:rustc-link-search=native=C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v12.0\\lib\\x64");
            println!("cargo:rustc-link-search=native=C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v11.8\\lib\\x64");
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Linux/macOS: Standard CUDA paths
        println!("cargo:rustc-link-search=native=/usr/local/cuda/lib64/");
        println!("cargo:rustc-link-search=native=/usr/lib/x86_64-linux-gnu/");
    }

    let builder = Builder::new("core")?;
    CargoAdapter::with_env_var("KERNEL_PTX_PATH").build(builder);
}