//! External CUDA backend using Danukeru's vanity_torv3_cuda executable
//!
//! This backend spawns an external CUDA process for GPU-accelerated generation.

use crate::onion::pubkey_to_onion;
use crate::{FILE_PREFIX, PUBKEY_PREFIX};
use crossbeam_channel::{Receiver, Sender};
use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
use curve25519_dalek::Scalar;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{BackendInfo, FoundKey, GeneratorError, Progress, SearchFilter};

/// External CUDA backend that spawns vanity_torv3_cuda executable
#[derive(Debug, Clone)]
pub struct ExternalCudaBackend {
    exe_path: PathBuf,
}

impl ExternalCudaBackend {
    /// Find and create external CUDA backend
    pub fn new() -> Result<Self, GeneratorError> {
        // Look for the CUDA executable in common locations
        let possible_paths = [
            // Same directory as our executable
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.join("vanity_torv3_cuda.exe"))),
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.join("vanity_torv3_cuda"))),
            // Current working directory
            Some(PathBuf::from("vanity_torv3_cuda.exe")),
            Some(PathBuf::from("vanity_torv3_cuda")),
            // Dist folder
            Some(PathBuf::from("dist/vanity_torv3_cuda.exe")),
        ];

        for path_opt in possible_paths.iter() {
            if let Some(path) = path_opt {
                if path.exists() {
                    return Ok(Self {
                        exe_path: path.clone(),
                    });
                }
            }
        }

        Err(GeneratorError::Cuda(
            "vanity_torv3_cuda executable not found. Please build it from https://github.com/Danukeru/torv3_vanity_addr_cuda".to_string()
        ))
    }

    /// Create with explicit path
    pub fn with_path(exe_path: PathBuf) -> Result<Self, GeneratorError> {
        if exe_path.exists() {
            Ok(Self { exe_path })
        } else {
            Err(GeneratorError::Cuda(format!(
                "CUDA executable not found at: {}",
                exe_path.display()
            )))
        }
    }

    /// Get backend information
    pub fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "External CUDA (vanity_torv3_cuda)".to_string(),
            estimated_speed: 100_000_000, // ~100M keys/sec estimate
        }
    }

    /// Start generation using external CUDA process
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

    /// Start generation using external CUDA process with additional filter
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

        // Create output directory if needed
        if !output_dir.exists() {
            std::fs::create_dir_all(&output_dir)
                .map_err(|e| GeneratorError::Io(e))?;
        }

        // Build command - convert prefixes to uppercase for the CUDA tool
        let mut cmd = Command::new(&self.exe_path);
        cmd.arg("-i"); // Enable rate reporting

        for prefix in &prefixes {
            cmd.arg(prefix.to_uppercase());
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Spawn the process
        let mut child = cmd.spawn().map_err(|e| {
            GeneratorError::Cuda(format!("Failed to start CUDA process: {}", e))
        })?;

        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        // Shared state
        let remaining: Arc<Mutex<HashSet<String>>> =
            Arc::new(Mutex::new(prefixes.iter().cloned().collect()));
        let counter = Arc::new(AtomicU64::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let start_time = Instant::now();
        let child_arc: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(Some(child)));

        // Prepare contains words (lowercase for case-insensitive matching)
        let contains_words: Vec<String> = filter.contains.iter().map(|w| w.to_lowercase()).collect();

        // Stop signal handler
        let stop_stopped = stopped.clone();
        let stop_child = child_arc.clone();
        std::thread::spawn(move || {
            if stop_rx.recv().is_ok() {
                stop_stopped.store(true, Ordering::SeqCst);
                // Kill the child process
                if let Some(mut child) = stop_child.lock().unwrap().take() {
                    let _ = child.kill();
                }
            }
        });

        // Progress reporting thread
        let progress_stopped = stopped.clone();
        let progress_counter = counter.clone();
        let progress_remaining = remaining.clone();
        std::thread::spawn(move || {
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

                if progress_remaining.lock().unwrap().is_empty() {
                    break;
                }

                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });

        // Read output from CUDA process
        for line in reader.lines() {
            if stopped.load(Ordering::Relaxed) {
                break;
            }

            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };

            // Parse hashrate updates (KEYRATE: X million keys/second)
            if line.starts_with("KEYRATE:") {
                if let Some(rate_str) = line.strip_prefix("KEYRATE:") {
                    if let Some(mkeys) = rate_str
                        .trim()
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<f64>().ok())
                    {
                        // Update counter based on rate
                        let elapsed = start_time.elapsed().as_secs_f64();
                        counter.store((mkeys * 1_000_000.0 * elapsed) as u64, Ordering::Relaxed);
                    }
                }
                continue;
            }

            // Skip non-hex lines
            if line.len() != 64 || !line.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }

            // Parse hex secret key (this is a scalar, not a seed!)
            // The CUDA tool outputs the ed25519 scalar directly after keccak + sc_reduce32
            let scalar_bytes = match hex_to_bytes(&line) {
                Some(sk) => sk,
                None => continue,
            };

            // Generate public key from scalar directly (not from seed!)
            // CUDA tool outputs scalar s, public key is A = s * B (base point)
            let scalar = Scalar::from_bytes_mod_order(scalar_bytes);
            let public_key_point = scalar * ED25519_BASEPOINT_POINT;
            let public_key_bytes = public_key_point.compress().to_bytes();
            let onion = pubkey_to_onion(&public_key_bytes);

            // Check which prefix matched
            let mut matched_prefix = None;
            for prefix in remaining.lock().unwrap().iter() {
                if onion.to_lowercase().starts_with(&prefix.to_lowercase()) {
                    matched_prefix = Some(prefix.clone());
                    break;
                }
            }

            // Check if address also contains all required words
            if matched_prefix.is_some() && !contains_words.is_empty() {
                let onion_lower = onion.to_lowercase();
                if !contains_words.iter().all(|word| onion_lower.contains(word)) {
                    // Prefix matched but contains filter failed - skip this address
                    matched_prefix = None;
                }
            }

            if let Some(prefix) = matched_prefix {
                // Remove from remaining
                remaining.lock().unwrap().remove(&prefix);

                // Create Tor hidden service directory structure
                // Strip .onion suffix for directory name
                let dir_name = onion.trim_end_matches(".onion");
                let hs_dir = output_dir.join(dir_name);

                if std::fs::create_dir_all(&hs_dir).is_ok() {
                    // 1. Write hostname file
                    let hostname_path = hs_dir.join("hostname");
                    if let Ok(mut f) = std::fs::File::create(&hostname_path) {
                        let _ = writeln!(f, "{}", onion);
                    }

                    // 2. Write hs_ed25519_public_key (32-byte tag + 32-byte pubkey)
                    let pubkey_path = hs_dir.join("hs_ed25519_public_key");
                    if let Ok(mut f) = std::fs::File::create(&pubkey_path) {
                        let _ = f.write_all(PUBKEY_PREFIX);
                        let _ = f.write_all(&public_key_bytes);
                    }

                    // 3. Write hs_ed25519_secret_key (32-byte tag + 64-byte expanded key)
                    // expanded_secret_key = scalar (32 bytes) || nonce_prefix (32 bytes)
                    // Since we only have the scalar from CUDA, use pubkey as nonce placeholder
                    let secret_path = hs_dir.join("hs_ed25519_secret_key");
                    if let Ok(mut f) = std::fs::File::create(&secret_path) {
                        let mut expanded = [0u8; 64];
                        expanded[..32].copy_from_slice(&scalar_bytes);
                        expanded[32..].copy_from_slice(&public_key_bytes);
                        let _ = f.write_all(FILE_PREFIX);
                        let _ = f.write_all(&expanded);
                    }

                    // 4. Create authorized_clients directory (required by Tor)
                    let _ = std::fs::create_dir_all(hs_dir.join("authorized_clients"));

                    let _ = result_tx.send(FoundKey {
                        prefix,
                        onion_address: onion,
                        key_path: hs_dir,
                    });
                }

                // Check if all done
                if remaining.lock().unwrap().is_empty() {
                    stopped.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }

        // Kill child process if still running
        if let Some(mut child) = child_arc.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        if stopped.load(Ordering::SeqCst) && !remaining.lock().unwrap().is_empty() {
            Err(GeneratorError::Stopped)
        } else {
            Ok(())
        }
    }
}

fn hex_to_bytes(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }

    let mut bytes = [0u8; 32];
    for i in 0..32 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}
