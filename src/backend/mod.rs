//! Backend abstraction for vanity address generation
//!
//! This module provides a trait-based abstraction over different computation
//! backends (CUDA GPU, CPU, and Hybrid CPU+GPU) for generating Tor v3 vanity addresses.

mod cpu;

#[cfg(feature = "cuda")]
mod cuda;

#[cfg(feature = "cuda")]
mod hybrid;

use crossbeam_channel::{Receiver, Sender};
use std::path::PathBuf;
use thiserror::Error;

pub use cpu::CpuBackend;

#[cfg(feature = "cuda")]
pub use cuda::CudaBackend;

#[cfg(feature = "cuda")]
pub use hybrid::HybridBackend;

/// Errors that can occur during generation
#[derive(Error, Debug)]
pub enum GeneratorError {
    #[error("CUDA error: {0}")]
    Cuda(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid prefix: {0}")]
    InvalidPrefix(String),

    #[error("Generation stopped by user")]
    Stopped,

    #[error("Channel error: {0}")]
    Channel(String),
}

/// Progress update from the generator
#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub keys_checked: u64,
    pub keys_per_sec: f64,
    pub elapsed_secs: f64,
}

/// A successfully found vanity key
#[derive(Debug, Clone)]
pub struct FoundKey {
    pub prefix: String,
    pub onion_address: String,
    pub key_path: PathBuf,
}

/// Information about a computation backend
#[derive(Debug, Clone)]
pub struct BackendInfo {
    pub name: String,
    pub estimated_speed: u64,
}

/// Available backend types
#[derive(Debug, Clone)]
pub enum Backend {
    Cpu(CpuBackend),
    #[cfg(feature = "cuda")]
    Cuda(CudaBackend),
    #[cfg(feature = "cuda")]
    Hybrid(HybridBackend),
}

/// Backend mode selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendMode {
    /// CPU only (always available)
    Cpu,
    /// GPU only (requires CUDA)
    #[cfg(feature = "cuda")]
    Cuda,
    /// CPU + GPU combined (requires CUDA)
    #[cfg(feature = "cuda")]
    Hybrid,
    /// Automatically select best available
    Auto,
}

impl Default for BackendMode {
    fn default() -> Self {
        BackendMode::Auto
    }
}

impl Backend {
    /// Get information about this backend
    pub fn info(&self) -> BackendInfo {
        match self {
            Backend::Cpu(b) => b.info(),
            #[cfg(feature = "cuda")]
            Backend::Cuda(b) => b.info(),
            #[cfg(feature = "cuda")]
            Backend::Hybrid(b) => b.info(),
        }
    }

    /// Start generation
    pub fn generate(
        &self,
        prefixes: Vec<String>,
        output_dir: PathBuf,
        progress_tx: Sender<Progress>,
        result_tx: Sender<FoundKey>,
        stop_rx: Receiver<()>,
    ) -> Result<(), GeneratorError> {
        match self {
            Backend::Cpu(b) => b.generate(prefixes, output_dir, progress_tx, result_tx, stop_rx),
            #[cfg(feature = "cuda")]
            Backend::Cuda(b) => b.generate(prefixes, output_dir, progress_tx, result_tx, stop_rx),
            #[cfg(feature = "cuda")]
            Backend::Hybrid(b) => b.generate(prefixes, output_dir, progress_tx, result_tx, stop_rx),
        }
    }
}

/// Select backend based on mode
pub fn select_backend_with_mode(mode: BackendMode) -> Backend {
    select_backend_with_config(mode, num_cpus::get())
}

/// Select backend based on mode with specific CPU thread count
pub fn select_backend_with_config(mode: BackendMode, cpu_threads: usize) -> Backend {
    match mode {
        BackendMode::Cpu => {
            let cpu = CpuBackend::with_threads(cpu_threads);
            print_backend_info(&cpu.info());
            Backend::Cpu(cpu)
        }
        #[cfg(feature = "cuda")]
        BackendMode::Cuda => {
            match CudaBackend::new() {
                Ok(cuda) => {
                    print_backend_info(&cuda.info());
                    Backend::Cuda(cuda)
                }
                Err(e) => {
                    eprintln!("CUDA not available ({}), falling back to CPU", e);
                    let cpu = CpuBackend::with_threads(cpu_threads);
                    print_backend_info(&cpu.info());
                    Backend::Cpu(cpu)
                }
            }
        }
        #[cfg(feature = "cuda")]
        BackendMode::Hybrid => {
            match HybridBackend::with_cpu_threads(cpu_threads) {
                Ok(hybrid) => {
                    print_backend_info(&hybrid.info());
                    Backend::Hybrid(hybrid)
                }
                Err(e) => {
                    eprintln!("Hybrid mode not available ({}), falling back to CPU", e);
                    let cpu = CpuBackend::with_threads(cpu_threads);
                    print_backend_info(&cpu.info());
                    Backend::Cpu(cpu)
                }
            }
        }
        BackendMode::Auto => select_backend_auto(cpu_threads),
    }
}

/// Select the best available backend with specific thread count
fn select_backend_auto(cpu_threads: usize) -> Backend {
    #[cfg(feature = "cuda")]
    {
        // Try hybrid first (CPU + GPU)
        match HybridBackend::with_cpu_threads(cpu_threads) {
            Ok(hybrid) => {
                print_backend_info(&hybrid.info());
                return Backend::Hybrid(hybrid);
            }
            Err(e) => {
                eprintln!("Hybrid mode not available: {}", e);
            }
        }

        // Try CUDA only
        match CudaBackend::new() {
            Ok(cuda) => {
                print_backend_info(&cuda.info());
                return Backend::Cuda(cuda);
            }
            Err(e) => {
                eprintln!("CUDA not available: {}", e);
            }
        }
    }

    // Fall back to CPU
    let cpu = CpuBackend::with_threads(cpu_threads);
    print_backend_info(&cpu.info());
    Backend::Cpu(cpu)
}

/// Select the best available backend automatically
///
/// Priority: Hybrid (CPU+GPU) > CUDA > CPU
pub fn select_backend() -> Backend {
    #[cfg(feature = "cuda")]
    {
        // Try hybrid first (CPU + GPU)
        match HybridBackend::new() {
            Ok(hybrid) => {
                print_backend_info(&hybrid.info());
                return Backend::Hybrid(hybrid);
            }
            Err(e) => {
                eprintln!("Hybrid mode not available: {}", e);
            }
        }

        // Try CUDA only
        match CudaBackend::new() {
            Ok(cuda) => {
                print_backend_info(&cuda.info());
                return Backend::Cuda(cuda);
            }
            Err(e) => {
                eprintln!("CUDA not available: {}", e);
            }
        }
    }

    // Fall back to CPU
    let cpu = CpuBackend::new();
    print_backend_info(&cpu.info());
    Backend::Cpu(cpu)
}

fn print_backend_info(info: &BackendInfo) {
    eprintln!("Backend: {}", info.name);
    eprintln!("Estimated speed: ~{} keys/sec", format_speed(info.estimated_speed));
}

/// Format speed for display
pub fn format_speed(speed: u64) -> String {
    if speed >= 1_000_000_000 {
        format!("{:.1}B", speed as f64 / 1_000_000_000.0)
    } else if speed >= 1_000_000 {
        format!("{:.1}M", speed as f64 / 1_000_000.0)
    } else if speed >= 1_000 {
        format!("{:.1}K", speed as f64 / 1_000.0)
    } else {
        format!("{}", speed)
    }
}
