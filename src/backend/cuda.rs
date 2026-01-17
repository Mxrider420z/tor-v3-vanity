//! CUDA GPU backend for high-speed key generation

use crate::onion::pubkey_to_onion;
use crate::FILE_PREFIX;
use crossbeam_channel::{Receiver, Sender};
use std::collections::HashSet;
use std::ffi::CString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rustacuda::launch;
use rustacuda::memory::{DeviceBox, DeviceBuffer};
use rustacuda::prelude::*;
use tor_v3_vanity_core as core;

use super::{BackendInfo, FoundKey, GeneratorError, Progress};

/// CUDA GPU backend for high-speed vanity address generation
#[derive(Debug, Clone)]
pub struct CudaBackend {
    device_count: usize,
    device_names: Vec<String>,
    estimated_speed: u64,
}

impl CudaBackend {
    /// Create a new CUDA backend, detecting available GPUs
    pub fn new() -> Result<Self, GeneratorError> {
        rustacuda::init(CudaFlags::empty())
            .map_err(|e| GeneratorError::Cuda(format!("Failed to initialize CUDA: {}", e)))?;

        let device_count = Device::num_devices()
            .map_err(|e| GeneratorError::Cuda(format!("Failed to count CUDA devices: {}", e)))?
            as usize;

        if device_count == 0 {
            return Err(GeneratorError::Cuda("No CUDA devices found".to_string()));
        }

        let mut device_names = Vec::new();
        let mut total_speed = 0u64;

        for i in 0..device_count {
            let device = Device::get_device(i as u32)
                .map_err(|e| GeneratorError::Cuda(format!("Failed to get device {}: {}", i, e)))?;

            let name = device.name()
                .map_err(|e| GeneratorError::Cuda(format!("Failed to get device name: {}", e)))?;

            // Estimate speed based on multiprocessor count
            let mp_count = device
                .get_attribute(rustacuda::device::DeviceAttribute::MultiprocessorCount)
                .unwrap_or(1) as u64;

            // Rough estimate: ~5M keys/sec per SM for GTX 10xx series
            let device_speed = mp_count * 5_000_000;
            total_speed += device_speed;

            device_names.push(name);
        }

        Ok(Self {
            device_count,
            device_names,
            estimated_speed: total_speed,
        })
    }

    /// Get backend information
    pub fn info(&self) -> BackendInfo {
        let name = if self.device_count == 1 {
            format!("CUDA ({})", self.device_names[0])
        } else {
            format!("CUDA ({} GPUs: {})", self.device_count, self.device_names.join(", "))
        };

        BackendInfo {
            name,
            estimated_speed: self.estimated_speed,
        }
    }

    /// Start vanity address generation on GPU
    pub fn generate(
        &self,
        prefixes: Vec<String>,
        output_dir: PathBuf,
        progress_tx: Sender<Progress>,
        result_tx: Sender<FoundKey>,
        stop_rx: Receiver<()>,
    ) -> Result<(), GeneratorError> {
        // Validate prefixes
        for prefix in &prefixes {
            if base32::decode(
                base32::Alphabet::Rfc4648Lower { padding: false },
                &format!("{}aa", prefix),
            )
            .is_none()
            {
                return Err(GeneratorError::InvalidPrefix(prefix.clone()));
            }
        }

        // Shared state
        let remaining: Arc<Mutex<HashSet<String>>> =
            Arc::new(Mutex::new(prefixes.clone().into_iter().collect()));
        let counter = Arc::new(AtomicU64::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let start_time = Instant::now();

        // Spawn a thread for each GPU
        let mut handles = Vec::new();
        let gpu_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        for device_idx in 0..self.device_count {
            let prefixes = prefixes.clone();
            let output_dir = output_dir.clone();
            let result_tx = result_tx.clone();
            let remaining = remaining.clone();
            let counter = counter.clone();
            let stopped = stopped.clone();
            let gpu_error = gpu_error.clone();

            let handle = std::thread::spawn(move || {
                if let Err(e) = Self::gpu_worker(
                    device_idx as u32,
                    prefixes,
                    output_dir,
                    result_tx,
                    remaining,
                    counter,
                    stopped.clone(),
                ) {
                    eprintln!("GPU {} error: {}", device_idx, e);
                    *gpu_error.lock().unwrap() = Some(format!("GPU {} error: {}", device_idx, e));
                    stopped.store(true, Ordering::SeqCst);
                }
            });

            handles.push(handle);
        }

        // Progress reporting thread
        let progress_stopped = stopped.clone();
        let progress_counter = counter.clone();
        let progress_remaining = remaining.clone();
        let progress_handle = std::thread::spawn(move || {
            while !progress_stopped.load(Ordering::Relaxed) {
                let keys_checked = progress_counter.load(Ordering::Relaxed);
                let elapsed = start_time.elapsed().as_secs_f64();
                let keys_per_sec = if elapsed > 0.0 {
                    keys_checked as f64 / elapsed
                } else {
                    0.0
                };

                let _ = progress_tx.send(Progress {
                    keys_checked,
                    keys_per_sec,
                    elapsed_secs: elapsed,
                });

                // Check if done
                if progress_remaining.lock().unwrap().is_empty() {
                    break;
                }

                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });

        // Stop signal handler
        let stop_stopped = stopped.clone();
        std::thread::spawn(move || {
            if stop_rx.recv().is_ok() {
                stop_stopped.store(true, Ordering::SeqCst);
            }
        });

        // Wait for completion or all prefixes found
        loop {
            if remaining.lock().unwrap().is_empty() || stopped.load(Ordering::SeqCst) {
                stopped.store(true, Ordering::SeqCst);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // Wait for all threads
        for handle in handles {
            let _ = handle.join();
        }
        let _ = progress_handle.join();

        // Check for GPU errors
        if let Some(err) = gpu_error.lock().unwrap().take() {
            return Err(GeneratorError::Cuda(err));
        }

        if stopped.load(Ordering::SeqCst) && !remaining.lock().unwrap().is_empty() {
            Err(GeneratorError::Stopped)
        } else {
            Ok(())
        }
    }

    fn gpu_worker(
        device_idx: u32,
        prefixes: Vec<String>,
        output_dir: PathBuf,
        result_tx: Sender<FoundKey>,
        remaining: Arc<Mutex<HashSet<String>>>,
        counter: Arc<AtomicU64>,
        stopped: Arc<AtomicBool>,
    ) -> Result<(), GeneratorError> {
        use rand::RngCore;

        let device = Device::get_device(device_idx)
            .map_err(|e| GeneratorError::Cuda(format!("Failed to get device: {}", e)))?;

        let _context = Context::create_and_push(
            ContextFlags::MAP_HOST | ContextFlags::SCHED_AUTO,
            device,
        )
        .map_err(|e| GeneratorError::Cuda(format!("Failed to create context: {}", e)))?;

        // Load PTX module
        let module_data = CString::new(include_str!(env!("KERNEL_PTX_PATH")))
            .map_err(|e| GeneratorError::Cuda(format!("Invalid PTX: {}", e)))?;

        let kernel = Module::load_from_string(&module_data)
            .map_err(|e| GeneratorError::Cuda(format!("Failed to load module: {}", e)))?;

        let function = kernel
            .get_function(std::ffi::CStr::from_bytes_with_nul(b"render\0").unwrap())
            .map_err(|e| GeneratorError::Cuda(format!("Failed to get function: {}", e)))?;

        let stream = Stream::new(StreamFlags::NON_BLOCKING, None)
            .map_err(|e| GeneratorError::Cuda(format!("Failed to create stream: {}", e)))?;

        // Set up GPU memory
        let mut seed = [0u8; 32];
        let mut gpu_seed = DeviceBuffer::from_slice(&seed)
            .map_err(|e| GeneratorError::Cuda(format!("Failed to allocate seed buffer: {}", e)))?;

        let mut byte_prefixes_owned: Vec<_> = prefixes
            .iter()
            .map(|s| BytePrefixOwned::from_str(s))
            .collect();

        let mut byte_prefixes: Vec<_> = byte_prefixes_owned
            .iter_mut()
            .map(|bp| bp.as_byte_prefix())
            .collect();

        let mut gpu_byte_prefixes = DeviceBuffer::from_slice(&byte_prefixes)
            .map_err(|e| GeneratorError::Cuda(format!("Failed to allocate prefix buffer: {}", e)))?;

        let mut params = DeviceBox::new(&core::KernelParams {
            seed: gpu_seed.as_device_ptr(),
            byte_prefixes: gpu_byte_prefixes.as_device_ptr(),
            byte_prefixes_len: gpu_byte_prefixes.len(),
        })
        .map_err(|e| GeneratorError::Cuda(format!("Failed to allocate params: {}", e)))?;

        // Calculate optimal thread/block configuration
        let fn_max_threads = function
            .get_attribute(rustacuda::function::FunctionAttribute::MaxThreadsPerBlock)
            .unwrap_or(256) as u32;

        let fn_registers = function
            .get_attribute(rustacuda::function::FunctionAttribute::NumRegisters)
            .unwrap_or(32) as u32;

        let gpu_max_threads = device
            .get_attribute(rustacuda::device::DeviceAttribute::MaxThreadsPerBlock)
            .unwrap_or(1024) as u32;

        let gpu_max_registers = device
            .get_attribute(rustacuda::device::DeviceAttribute::MaxRegistersPerBlock)
            .unwrap_or(65536) as u32;

        let gpu_cores = device
            .get_attribute(rustacuda::device::DeviceAttribute::MultiprocessorCount)
            .unwrap_or(1) as u32;

        let threads = *[
            fn_max_threads,
            gpu_max_threads,
            if fn_registers > 0 { gpu_max_registers / fn_registers } else { 256 },
        ]
        .iter()
        .min()
        .unwrap();

        let blocks = gpu_cores * gpu_max_threads / threads;

        let mut rng = rand::thread_rng();

        // Main generation loop
        while !stopped.load(Ordering::Relaxed) {
            // Check if all prefixes found
            if remaining.lock().unwrap().is_empty() {
                break;
            }

            // Generate new random seed
            rng.fill_bytes(&mut seed);
            gpu_seed.copy_from(&seed)
                .map_err(|e| GeneratorError::Cuda(format!("Failed to copy seed: {}", e)))?;

            // Launch kernel
            unsafe {
                launch!(kernel.render<<<blocks, threads, 0, stream>>>(params.as_device_ptr()))
                    .map_err(|e| GeneratorError::Cuda(format!("Kernel launch failed: {}", e)))?;
            }

            stream.synchronize()
                .map_err(|e| GeneratorError::Cuda(format!("Stream sync failed: {}", e)))?;

            // Check results
            gpu_byte_prefixes.copy_to(&mut byte_prefixes)
                .map_err(|e| GeneratorError::Cuda(format!("Failed to copy results: {}", e)))?;

            for (i, prefix) in byte_prefixes_owned.iter_mut().enumerate() {
                let mut success = false;
                prefix.success.copy_to(&mut success).ok();

                if success {
                    prefix.success.copy_from(&false).ok();

                    let mut out = [0u8; 32];
                    prefix.out.copy_to(&mut out).ok();

                    // Generate full keypair from seed
                    let esk: ed25519_dalek::hazmat::ExpandedSecretKey =
                        ed25519_dalek::hazmat::ExpandedSecretKey::from_bytes(&{
                            let mut expanded = [0u8; 64];
                            // The GPU outputs the scalar, we need to reconstruct
                            expanded[..32].copy_from_slice(&out);
                            expanded
                        });

                    // Actually we need to re-derive from the original approach
                    // The GPU kernel outputs a modified seed that produces the match
                    let signing_key = ed25519_dalek::SigningKey::from_bytes(&out);
                    let verifying_key = signing_key.verifying_key();
                    let onion = pubkey_to_onion(&verifying_key.to_bytes());

                    let prefix_str = &prefixes[i];

                    // Remove from remaining
                    remaining.lock().unwrap().remove(prefix_str);

                    // Save key file
                    let key_path = output_dir.join(&onion);
                    if let Ok(mut f) = std::fs::File::create(&key_path) {
                        let expanded = signing_key.to_keypair_bytes();
                        let _ = f.write_all(FILE_PREFIX);
                        let _ = f.write_all(&expanded);
                        let _ = f.flush();

                        let _ = result_tx.send(FoundKey {
                            prefix: prefix_str.clone(),
                            onion_address: onion,
                            key_path,
                        });
                    }
                }
            }

            counter.fetch_add((threads * blocks) as u64, Ordering::Relaxed);
        }

        Ok(())
    }
}

/// GPU-side prefix matching structure
struct BytePrefixOwned {
    byte_prefix: DeviceBuffer<u8>,
    last_byte_idx: usize,
    last_byte_mask: u8,
    out: DeviceBuffer<u8>,
    success: DeviceBox<bool>,
}

impl BytePrefixOwned {
    fn from_str(s: &str) -> Self {
        let byte_prefix = base32::decode(
            base32::Alphabet::Rfc4648Lower { padding: false },
            &format!("{}aa", s),
        )
        .expect("prefix must be base32");

        let mut last_byte_idx = 5 * s.len() / 8;
        if last_byte_idx > 0 {
            last_byte_idx -= 1;
        }

        let n_bits = (5 * s.len()) % 8;
        let last_byte_mask = ((1u16 << n_bits) - 1) as u8;
        let last_byte_mask = last_byte_mask << (8 - n_bits);

        if last_byte_mask > 0 && last_byte_idx < byte_prefix.len() - 1 {
            last_byte_idx += 1;
        }

        let gpu_byte_prefix = DeviceBuffer::from_slice(&byte_prefix).unwrap();
        let out = [0u8; 32];
        let gpu_out = DeviceBuffer::from_slice(&out).unwrap();
        let success = false;
        let gpu_success = DeviceBox::new(&success).unwrap();

        BytePrefixOwned {
            byte_prefix: gpu_byte_prefix,
            last_byte_idx,
            last_byte_mask,
            out: gpu_out,
            success: gpu_success,
        }
    }

    fn as_byte_prefix(&mut self) -> core::BytePrefix {
        core::BytePrefix {
            byte_prefix: self.byte_prefix.as_device_ptr(),
            byte_prefix_len: self.byte_prefix.len(),
            last_byte_idx: self.last_byte_idx,
            last_byte_mask: self.last_byte_mask,
            out: self.out.as_device_ptr(),
            success: self.success.as_device_ptr(),
        }
    }
}
