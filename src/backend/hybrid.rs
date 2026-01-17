//! Hybrid CPU+GPU backend for maximum throughput
//!
//! Runs both CPU and GPU backends simultaneously, combining their speeds.

use crate::onion::pubkey_to_onion;
use crate::FILE_PREFIX;
use crossbeam_channel::{Receiver, Sender};
use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::cpu::CpuBackend;
use super::cuda::CudaBackend;
use super::{BackendInfo, FoundKey, GeneratorError, Progress};

/// Hybrid backend that runs CPU and GPU in parallel
#[derive(Debug, Clone)]
pub struct HybridBackend {
    cpu_threads: usize,
    gpu_info: String,
    cpu_speed: u64,
    gpu_speed: u64,
}

impl HybridBackend {
    /// Create a new hybrid backend with default settings
    pub fn new() -> Result<Self, GeneratorError> {
        Self::with_cpu_threads(num_cpus::get())
    }

    /// Create a hybrid backend with specified CPU thread count
    pub fn with_cpu_threads(cpu_threads: usize) -> Result<Self, GeneratorError> {
        // Verify CUDA is available
        let cuda = CudaBackend::new()?;
        let cuda_info = cuda.info();

        let cpu = CpuBackend::with_threads(cpu_threads);
        let cpu_info = cpu.info();

        Ok(Self {
            cpu_threads,
            gpu_info: cuda_info.name,
            cpu_speed: cpu_info.estimated_speed,
            gpu_speed: cuda_info.estimated_speed,
        })
    }

    /// Get backend information
    pub fn info(&self) -> BackendInfo {
        BackendInfo {
            name: format!(
                "Hybrid: {} + CPU ({} threads)",
                self.gpu_info, self.cpu_threads
            ),
            estimated_speed: self.cpu_speed + self.gpu_speed,
        }
    }

    /// Start generation on both CPU and GPU simultaneously
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

        // Shared state across CPU and GPU
        let remaining: Arc<Mutex<HashSet<String>>> =
            Arc::new(Mutex::new(prefixes.clone().into_iter().collect()));
        let cpu_counter = Arc::new(AtomicU64::new(0));
        let gpu_counter = Arc::new(AtomicU64::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let start_time = Instant::now();

        // Create stop channels for each backend
        let (cpu_stop_tx, cpu_stop_rx) = crossbeam_channel::bounded(1);
        let (gpu_stop_tx, gpu_stop_rx) = crossbeam_channel::bounded(1);

        // Spawn CPU worker threads
        let cpu_handles = self.spawn_cpu_workers(
            prefixes.clone(),
            output_dir.clone(),
            result_tx.clone(),
            remaining.clone(),
            cpu_counter.clone(),
            stopped.clone(),
            cpu_stop_rx,
        );

        // Spawn GPU worker threads
        let gpu_handle = self.spawn_gpu_workers(
            prefixes,
            output_dir,
            result_tx.clone(),
            remaining.clone(),
            gpu_counter.clone(),
            stopped.clone(),
            gpu_stop_rx,
        );

        // Progress reporting
        let progress_stopped = stopped.clone();
        let progress_cpu_counter = cpu_counter.clone();
        let progress_gpu_counter = gpu_counter.clone();
        let progress_remaining = remaining.clone();

        let progress_handle = std::thread::spawn(move || {
            while !progress_stopped.load(Ordering::Relaxed) {
                let cpu_keys = progress_cpu_counter.load(Ordering::Relaxed);
                let gpu_keys = progress_gpu_counter.load(Ordering::Relaxed);
                let total_keys = cpu_keys + gpu_keys;

                let elapsed = start_time.elapsed().as_secs_f64();
                let keys_per_sec = if elapsed > 0.0 {
                    total_keys as f64 / elapsed
                } else {
                    0.0
                };

                let _ = progress_tx.send(Progress {
                    keys_checked: total_keys,
                    keys_per_sec,
                    elapsed_secs: elapsed,
                });

                if progress_remaining.lock().unwrap().is_empty() {
                    break;
                }

                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });

        // Handle stop signal
        let stop_stopped = stopped.clone();
        std::thread::spawn(move || {
            if stop_rx.recv().is_ok() {
                stop_stopped.store(true, Ordering::SeqCst);
                let _ = cpu_stop_tx.send(());
                let _ = gpu_stop_tx.send(());
            }
        });

        // Wait for completion
        loop {
            if remaining.lock().unwrap().is_empty() || stopped.load(Ordering::SeqCst) {
                stopped.store(true, Ordering::SeqCst);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        // Wait for all workers
        for handle in cpu_handles {
            let _ = handle.join();
        }
        if let Some(handle) = gpu_handle {
            let _ = handle.join();
        }
        let _ = progress_handle.join();

        if stopped.load(Ordering::SeqCst) && !remaining.lock().unwrap().is_empty() {
            Err(GeneratorError::Stopped)
        } else {
            Ok(())
        }
    }

    fn spawn_cpu_workers(
        &self,
        prefixes: Vec<String>,
        output_dir: PathBuf,
        result_tx: Sender<FoundKey>,
        remaining: Arc<Mutex<HashSet<String>>>,
        counter: Arc<AtomicU64>,
        stopped: Arc<AtomicBool>,
        _stop_rx: Receiver<()>,
    ) -> Vec<std::thread::JoinHandle<()>> {
        let mut handles = Vec::new();

        for _ in 0..self.cpu_threads {
            let prefixes = prefixes.clone();
            let output_dir = output_dir.clone();
            let result_tx = result_tx.clone();
            let remaining = remaining.clone();
            let counter = counter.clone();
            let stopped = stopped.clone();

            let handle = std::thread::spawn(move || {
                Self::cpu_worker(
                    prefixes,
                    output_dir,
                    result_tx,
                    remaining,
                    counter,
                    stopped,
                );
            });

            handles.push(handle);
        }

        handles
    }

    fn cpu_worker(
        prefixes: Vec<String>,
        output_dir: PathBuf,
        result_tx: Sender<FoundKey>,
        remaining: Arc<Mutex<HashSet<String>>>,
        counter: Arc<AtomicU64>,
        stopped: Arc<AtomicBool>,
    ) {
        while !stopped.load(Ordering::Relaxed) {
            if remaining.lock().unwrap().is_empty() {
                break;
            }

            // Generate random seed
            let seed: [u8; 32] = rand::random();

            // Create keypair
            let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
            let verifying_key = signing_key.verifying_key();
            let pubkey_bytes: [u8; 32] = verifying_key.to_bytes();

            // Generate onion address
            let onion = pubkey_to_onion(&pubkey_bytes);

            // Check against remaining prefixes
            let remaining_guard = remaining.lock().unwrap();
            let mut found_prefix = None;
            for prefix in remaining_guard.iter() {
                if onion.starts_with(prefix) {
                    found_prefix = Some(prefix.clone());
                    break;
                }
            }
            drop(remaining_guard);

            // If found, save and notify
            if let Some(prefix) = found_prefix {
                remaining.lock().unwrap().remove(&prefix);

                let key_path = output_dir.join(&onion);
                if let Ok(mut f) = std::fs::File::create(&key_path) {
                    let expanded = signing_key.to_keypair_bytes();
                    let _ = f.write_all(FILE_PREFIX);
                    let _ = f.write_all(&expanded);
                    let _ = f.flush();

                    let _ = result_tx.send(FoundKey {
                        prefix,
                        onion_address: onion,
                        key_path,
                    });
                }
            }

            counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn spawn_gpu_workers(
        &self,
        prefixes: Vec<String>,
        output_dir: PathBuf,
        result_tx: Sender<FoundKey>,
        remaining: Arc<Mutex<HashSet<String>>>,
        counter: Arc<AtomicU64>,
        stopped: Arc<AtomicBool>,
        _stop_rx: Receiver<()>,
    ) -> Option<std::thread::JoinHandle<()>> {
        // Spawn GPU in a separate thread
        let handle = std::thread::spawn(move || {
            // Create internal channels for GPU backend
            let (internal_progress_tx, _internal_progress_rx) = crossbeam_channel::unbounded();
            let (internal_stop_tx, internal_stop_rx) = crossbeam_channel::bounded(1);

            // Create a wrapper that updates our shared counter
            let gpu_result_tx = result_tx;
            let gpu_remaining = remaining;
            let gpu_counter = counter;
            let gpu_stopped = stopped;

            // We'll run the CUDA backend's internal logic here
            if let Ok(cuda) = CudaBackend::new() {
                // Monitor for stop
                let monitor_stopped = gpu_stopped.clone();
                std::thread::spawn(move || {
                    while !monitor_stopped.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    let _ = internal_stop_tx.send(());
                });

                let _ = cuda.generate(
                    prefixes,
                    output_dir,
                    internal_progress_tx,
                    gpu_result_tx,
                    internal_stop_rx,
                );
            }
        });

        Some(handle)
    }
}
