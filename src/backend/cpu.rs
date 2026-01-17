//! CPU backend using Rayon for parallel processing

use crate::onion::pubkey_to_onion;
use crate::{FILE_PREFIX, PUBKEY_PREFIX};
use crossbeam_channel::{Receiver, Sender};
use rayon::prelude::*;
use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{BackendInfo, FoundKey, GeneratorError, Progress, SearchFilter};

/// CPU backend using Rayon for parallel key generation
#[derive(Debug, Clone)]
pub struct CpuBackend {
    thread_count: usize,
}

impl CpuBackend {
    /// Create a new CPU backend using all available cores
    pub fn new() -> Self {
        Self {
            thread_count: num_cpus::get(),
        }
    }

    /// Create a CPU backend with a specific thread count
    pub fn with_threads(thread_count: usize) -> Self {
        Self { thread_count }
    }

    /// Get backend information
    pub fn info(&self) -> BackendInfo {
        BackendInfo {
            name: format!("CPU ({} threads)", self.thread_count),
            // Estimate ~400K-600K keys/sec per thread (conservative)
            estimated_speed: (self.thread_count as u64) * 500_000,
        }
    }

    /// Start vanity address generation
    pub fn generate(
        &self,
        prefixes: Vec<String>,
        output_dir: PathBuf,
        progress_tx: Sender<Progress>,
        result_tx: Sender<FoundKey>,
        stop_rx: Receiver<()>,
    ) -> Result<(), GeneratorError> {
        self.generate_with_filter(prefixes, output_dir, progress_tx, result_tx, stop_rx, SearchFilter::default())
    }

    /// Start vanity address generation with additional filter
    pub fn generate_with_filter(
        &self,
        prefixes: Vec<String>,
        output_dir: PathBuf,
        progress_tx: Sender<Progress>,
        result_tx: Sender<FoundKey>,
        stop_rx: Receiver<()>,
        filter: SearchFilter,
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

        // Set up thread pool
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.thread_count)
            .build()
            .map_err(|e| GeneratorError::Channel(e.to_string()))?;

        // Shared state
        let remaining: Arc<Mutex<HashSet<String>>> =
            Arc::new(Mutex::new(prefixes.into_iter().collect()));
        let counter = Arc::new(AtomicU64::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let start_time = Instant::now();

        // Prepare contains words (lowercase for case-insensitive matching)
        let contains_words: Arc<Vec<String>> = Arc::new(
            filter.contains.iter().map(|w| w.to_lowercase()).collect()
        );

        // Batch size per iteration
        const BATCH_SIZE: usize = 10_000;

        pool.install(|| {
            loop {
                // Check stop signal
                if stop_rx.try_recv().is_ok() {
                    stopped.store(true, Ordering::SeqCst);
                    break;
                }

                if stopped.load(Ordering::SeqCst) {
                    break;
                }

                // Check if all prefixes found
                if remaining.lock().unwrap().is_empty() {
                    break;
                }

                // Process batch in parallel
                (0..BATCH_SIZE).into_par_iter().for_each(|_| {
                    if stopped.load(Ordering::Relaxed) {
                        return;
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

                    // Check if address also contains all required words
                    if found_prefix.is_some() && !contains_words.is_empty() {
                        let onion_lower = onion.to_lowercase();
                        if !contains_words.iter().all(|word| onion_lower.contains(word)) {
                            // Prefix matched but contains filter failed - skip this address
                            found_prefix = None;
                        }
                    }

                    // If found, save and notify
                    if let Some(prefix) = found_prefix {
                        // Remove from remaining
                        remaining.lock().unwrap().remove(&prefix);

                        // Create Tor hidden service directory structure
                        let dir_name = onion.trim_end_matches(".onion");
                        let hs_dir = output_dir.join(dir_name);

                        if std::fs::create_dir_all(&hs_dir).is_ok() {
                            // 1. Write hostname file
                            let hostname_path = hs_dir.join("hostname");
                            if let Ok(mut f) = std::fs::File::create(&hostname_path) {
                                let _ = writeln!(f, "{}", onion);
                            }

                            // 2. Write hs_ed25519_public_key
                            let pubkey_path = hs_dir.join("hs_ed25519_public_key");
                            if let Ok(mut f) = std::fs::File::create(&pubkey_path) {
                                let _ = f.write_all(PUBKEY_PREFIX);
                                let _ = f.write_all(&pubkey_bytes);
                            }

                            // 3. Write hs_ed25519_secret_key
                            // For CPU backend, we have the seed, so create proper expanded key
                            let secret_path = hs_dir.join("hs_ed25519_secret_key");
                            if let Ok(mut f) = std::fs::File::create(&secret_path) {
                                // Tor expects: scalar (clamped) || nonce_prefix
                                // ed25519_dalek derives these from SHA512(seed)
                                use sha2::{Sha512, Digest};
                                let hash = Sha512::digest(&seed);
                                let mut expanded = [0u8; 64];
                                expanded.copy_from_slice(&hash);
                                // Clamp the scalar part (first 32 bytes)
                                expanded[0] &= 248;
                                expanded[31] &= 127;
                                expanded[31] |= 64;
                                let _ = f.write_all(FILE_PREFIX);
                                let _ = f.write_all(&expanded);
                            }

                            // 4. Create authorized_clients directory
                            let _ = std::fs::create_dir_all(hs_dir.join("authorized_clients"));

                            // Send result
                            let _ = result_tx.send(FoundKey {
                                prefix,
                                onion_address: onion,
                                key_path: hs_dir,
                            });
                        }
                    }

                    counter.fetch_add(1, Ordering::Relaxed);
                });

                // Send progress update
                let keys_checked = counter.load(Ordering::Relaxed);
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
            }
        });

        if stopped.load(Ordering::SeqCst) {
            Err(GeneratorError::Stopped)
        } else {
            Ok(())
        }
    }
}

impl Default for CpuBackend {
    fn default() -> Self {
        Self::new()
    }
}
