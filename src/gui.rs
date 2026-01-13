//! Tor v3 Vanity Address Generator - GUI
//!
//! A simple egui-based GUI for generating vanity Tor v3 onion addresses
//! using NVIDIA GPU acceleration.

use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use failure::Error;
use sha3::{Digest, Sha3_256};

// Re-use the core CUDA functionality
use tor_v3_vanity_core as core;

/// Pattern position for matching
#[derive(Clone, Copy, PartialEq, Debug)]
enum Position {
    Prefix,
    Suffix,
    Anywhere,
}

impl Position {
    fn label(&self) -> &'static str {
        match self {
            Position::Prefix => "Prefix",
            Position::Suffix => "Suffix",
            Position::Anywhere => "Anywhere",
        }
    }
}

/// A single pattern entry with position
#[derive(Clone)]
struct PatternEntry {
    pattern: String,
    position: Position,
}

impl Default for PatternEntry {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            position: Position::Prefix,
        }
    }
}

/// A found result
#[derive(Clone)]
struct FoundResult {
    address: String,
    file_path: PathBuf,
    timestamp: String,
}

/// Shared state between GUI and worker threads
struct SharedState {
    tries: AtomicU64,
    rate: Mutex<f64>,
    running: AtomicBool,
    results: Mutex<Vec<FoundResult>>,
    status: Mutex<String>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            tries: AtomicU64::new(0),
            rate: Mutex::new(0.0),
            running: AtomicBool::new(false),
            results: Mutex::new(Vec::new()),
            status: Mutex::new("Ready".to_string()),
        }
    }
}

/// Convert a public key to an onion address
fn pubkey_to_onion(pubkey: &[u8; 32]) -> String {
    let mut hasher = Sha3_256::new();
    hasher.update(b".onion checksum");
    hasher.update(pubkey);
    hasher.update(&[3]);
    let mut onion = [0u8; 35];
    onion[..32].clone_from_slice(pubkey);
    onion[32..34].clone_from_slice(&hasher.finalize()[..2]);
    onion[34] = 3;
    format!(
        "{}.onion",
        base32::encode(base32::Alphabet::RFC4648 { padding: false }, &onion).to_lowercase()
    )
}

/// Check if a pattern matches an onion address at the given position
fn pattern_matches(onion: &str, pattern: &str, position: Position) -> bool {
    let pattern_lower = pattern.to_lowercase();
    let onion_name = onion.trim_end_matches(".onion");

    match position {
        Position::Prefix => onion_name.starts_with(&pattern_lower),
        Position::Suffix => onion_name.ends_with(&pattern_lower),
        Position::Anywhere => onion_name.contains(&pattern_lower),
    }
}

/// Main application state
struct VanityApp {
    // Input state
    patterns: Vec<PatternEntry>,
    output_dir: String,

    // Shared state with worker
    shared: Arc<SharedState>,

    // Communication channels
    stop_sender: Option<Sender<()>>,
    result_receiver: Option<Receiver<FoundResult>>,

    // UI state
    error_message: Option<String>,
    start_time: Option<Instant>,
}

impl Default for VanityApp {
    fn default() -> Self {
        let default_output = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("vanity_keys");

        Self {
            patterns: vec![PatternEntry::default()],
            output_dir: default_output.to_string_lossy().to_string(),
            shared: Arc::new(SharedState::default()),
            stop_sender: None,
            result_receiver: None,
            error_message: None,
            start_time: None,
        }
    }
}

impl VanityApp {
    /// Validate patterns before starting
    fn validate_patterns(&self) -> Result<Vec<(String, Position)>, String> {
        let valid_chars: std::collections::HashSet<char> =
            "abcdefghijklmnopqrstuvwxyz234567".chars().collect();

        let mut valid_patterns = Vec::new();

        for entry in &self.patterns {
            let pattern = entry.pattern.trim().to_lowercase();
            if pattern.is_empty() {
                continue;
            }

            // Check for invalid characters
            for c in pattern.chars() {
                if !valid_chars.contains(&c) {
                    return Err(format!(
                        "Invalid character '{}' in pattern '{}'. Only a-z and 2-7 are allowed.",
                        c, entry.pattern
                    ));
                }
            }

            // Check length (max useful length is ~10 for reasonable search times)
            if pattern.len() > 12 {
                return Err(format!(
                    "Pattern '{}' is too long (max 12 chars). This would take years to find.",
                    entry.pattern
                ));
            }

            valid_patterns.push((pattern, entry.position));
        }

        if valid_patterns.is_empty() {
            return Err("Please enter at least one pattern".to_string());
        }

        Ok(valid_patterns)
    }

    /// Estimate time based on pattern length
    fn estimate_time(patterns: &[(String, Position)]) -> String {
        if patterns.is_empty() {
            return "N/A".to_string();
        }

        // Find the longest pattern (determines search time)
        let max_len = patterns.iter().map(|(p, _)| p.len()).max().unwrap_or(0);

        // Rough estimates based on typical GPU performance (~50M keys/sec)
        let estimates = [
            (1, "< 1 second"),
            (2, "< 1 second"),
            (3, "< 1 second"),
            (4, "~2 seconds"),
            (5, "~1 minute"),
            (6, "~30 minutes"),
            (7, "~16 hours"),
            (8, "~3 weeks"),
            (9, "~2 years"),
            (10, "~64 years"),
        ];

        estimates.get(max_len.saturating_sub(1))
            .map(|(_, s)| s.to_string())
            .unwrap_or_else(|| "Very long time".to_string())
    }

    /// Start the generation process
    fn start_generation(&mut self, patterns: Vec<(String, Position)>) {
        // Create output directory
        let output_path = PathBuf::from(&self.output_dir);
        if let Err(e) = std::fs::create_dir_all(&output_path) {
            self.error_message = Some(format!("Failed to create output directory: {}", e));
            return;
        }

        // Set up channels
        let (stop_tx, stop_rx) = bounded::<()>(1);
        let (result_tx, result_rx) = unbounded::<FoundResult>();

        self.stop_sender = Some(stop_tx);
        self.result_receiver = Some(result_rx);

        // Reset state
        self.shared.tries.store(0, Ordering::Relaxed);
        *self.shared.rate.lock().unwrap() = 0.0;
        self.shared.running.store(true, Ordering::Relaxed);
        *self.shared.status.lock().unwrap() = "Starting...".to_string();
        self.start_time = Some(Instant::now());
        self.error_message = None;

        // Clone for thread
        let shared = Arc::clone(&self.shared);
        let output_dir = output_path.clone();

        // Extract prefix patterns only for now (position matching done in post-processing)
        let prefix_patterns: Vec<String> = patterns.iter()
            .filter(|(_, pos)| *pos == Position::Prefix)
            .map(|(p, _)| p.clone())
            .collect();

        let all_patterns = patterns;

        // Spawn worker thread
        std::thread::spawn(move || {
            // Initialize CUDA
            *shared.status.lock().unwrap() = "Initializing CUDA...".to_string();

            if let Err(e) = run_cuda_generation(
                &prefix_patterns,
                &all_patterns,
                output_dir,
                stop_rx,
                result_tx,
                Arc::clone(&shared),
            ) {
                *shared.status.lock().unwrap() = format!("Error: {}", e);
            }

            shared.running.store(false, Ordering::Relaxed);
        });
    }

    /// Stop the generation process
    fn stop_generation(&mut self) {
        if let Some(sender) = self.stop_sender.take() {
            let _ = sender.send(());
        }
        *self.shared.status.lock().unwrap() = "Stopping...".to_string();
    }
}

/// Run the CUDA generation loop
fn run_cuda_generation(
    prefix_patterns: &[String],
    all_patterns: &[(String, Position)],
    output_dir: PathBuf,
    stop_rx: Receiver<()>,
    result_tx: Sender<FoundResult>,
    shared: Arc<SharedState>,
) -> Result<(), Error> {
    use rustacuda::launch;
    use rustacuda::memory::{DeviceBox, DeviceBuffer};
    use rustacuda::prelude::*;
    use std::ffi::CString;

    // Initialize CUDA
    rustacuda::init(CudaFlags::empty())?;

    let device = Device::get_device(0)?;
    let _context = Context::create_and_push(
        ContextFlags::MAP_HOST | ContextFlags::SCHED_AUTO,
        device,
    )?;

    *shared.status.lock().unwrap() = "Loading CUDA kernel...".to_string();

    // Load the PTX module
    let module_data = CString::new(include_str!(env!("KERNEL_PTX_PATH")))?;
    let kernel = Module::load_from_string(&module_data)?;
    let function = kernel.get_function(
        std::ffi::CStr::from_bytes_with_nul(b"render\0").unwrap()
    )?;

    // Create stream
    let stream = Stream::new(StreamFlags::NON_BLOCKING, None)?;

    // Set up seed buffer
    let mut seed = [0u8; 32];
    let mut gpu_seed = DeviceBuffer::from_slice(&seed)?;

    // Set up prefix matching (only prefix patterns for GPU matching)
    let mut byte_prefixes_owned: Vec<_> = prefix_patterns
        .iter()
        .map(|s| BytePrefixOwned::from_str(s))
        .collect();

    // If no prefix patterns, we still need at least one for the kernel
    // In this case, we'll match everything and filter on CPU
    if byte_prefixes_owned.is_empty() {
        byte_prefixes_owned.push(BytePrefixOwned::from_str(""));
    }

    let mut byte_prefixes: Vec<_> = byte_prefixes_owned
        .iter_mut()
        .map(|bp| bp.as_byte_prefix())
        .collect();
    let mut gpu_byte_prefixes = DeviceBuffer::from_slice(&byte_prefixes)?;

    // Create kernel parameters
    let mut params = DeviceBox::new(&core::KernelParams {
        seed: gpu_seed.as_device_ptr(),
        byte_prefixes: gpu_byte_prefixes.as_device_ptr(),
        byte_prefixes_len: gpu_byte_prefixes.len(),
    })?;

    // Calculate thread/block configuration
    let fn_max_threads = function
        .get_attribute(rustacuda::function::FunctionAttribute::MaxThreadsPerBlock)? as u32;
    let fn_registers = function
        .get_attribute(rustacuda::function::FunctionAttribute::NumRegisters)? as u32;
    let gpu_max_threads = device
        .get_attribute(rustacuda::device::DeviceAttribute::MaxThreadsPerBlock)? as u32;
    let gpu_max_registers = device
        .get_attribute(rustacuda::device::DeviceAttribute::MaxRegistersPerBlock)? as u32;
    let gpu_cores = device
        .get_attribute(rustacuda::device::DeviceAttribute::MultiprocessorCount)? as u32;

    let threads = *[
        fn_max_threads,
        gpu_max_threads,
        gpu_max_registers / fn_registers.max(1),
    ]
    .iter()
    .min()
    .unwrap();
    let blocks = gpu_cores * gpu_max_threads / threads;

    *shared.status.lock().unwrap() = format!(
        "Running on {} ({} threads, {} blocks)",
        device.name()?,
        threads,
        blocks
    );

    let mut csprng = rand::thread_rng();
    let start_time = Instant::now();
    let mut last_update = Instant::now();

    // Main generation loop
    loop {
        // Check for stop signal
        if stop_rx.try_recv().is_ok() {
            *shared.status.lock().unwrap() = "Stopped by user".to_string();
            break;
        }

        // Generate new random seed
        use rand::RngCore;
        csprng.fill_bytes(&mut seed);
        gpu_seed.copy_from(&seed)?;

        // Launch kernel
        unsafe {
            launch!(kernel.render<<<blocks, threads, 0, stream>>>(params.as_device_ptr()))?;
        }

        // Wait for kernel completion
        stream.synchronize()?;

        // Check results
        gpu_byte_prefixes.copy_to(&mut byte_prefixes)?;

        for prefix in &mut byte_prefixes_owned {
            let mut success = false;
            prefix.success.copy_to(&mut success)?;

            if success {
                prefix.success.copy_from(&false)?;
                let mut out = [0u8; 32];
                prefix.out.copy_to(&mut out)?;

                // Generate the full address
                let esk: ed25519_dalek::ExpandedSecretKey =
                    (&ed25519_dalek::SecretKey::from_bytes(&out).unwrap()).into();
                let pk: ed25519_dalek::PublicKey = (&esk).into();
                let onion = pubkey_to_onion(pk.as_bytes());

                // Check if it matches any of our patterns (including non-prefix)
                let matches_pattern = all_patterns.iter().any(|(pattern, position)| {
                    pattern_matches(&onion, pattern, *position)
                });

                if matches_pattern {
                    // Save the key
                    let file_path = output_dir.join(&onion);
                    if let Ok(mut f) = std::fs::File::create(&file_path) {
                        use std::io::Write;
                        let file_prefix = b"== ed25519v1-secret: type0 ==\0\0\0";
                        let _ = f.write_all(file_prefix);
                        let _ = f.write_all(&esk.to_bytes());
                    }

                    // Send result to GUI
                    let _ = result_tx.send(FoundResult {
                        address: onion,
                        file_path,
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    });
                }
            }
        }

        // Update statistics
        let tries = shared.tries.fetch_add(
            threads as u64 * blocks as u64,
            Ordering::Relaxed
        ) + threads as u64 * blocks as u64;

        if last_update.elapsed() > Duration::from_millis(500) {
            let elapsed = start_time.elapsed().as_secs_f64();
            let rate = tries as f64 / elapsed;
            *shared.rate.lock().unwrap() = rate;
            last_update = Instant::now();
        }
    }

    Ok(())
}

/// Owned byte prefix for GPU matching
struct BytePrefixOwned {
    byte_prefix: rustacuda::memory::DeviceBuffer<u8>,
    last_byte_idx: usize,
    last_byte_mask: u8,
    out: rustacuda::memory::DeviceBuffer<u8>,
    success: rustacuda::memory::DeviceBox<bool>,
}

impl BytePrefixOwned {
    fn from_str(s: &str) -> Self {
        let byte_prefix = if s.is_empty() {
            vec![0u8; 1]
        } else {
            base32::decode(
                base32::Alphabet::RFC4648 { padding: false },
                &format!("{}aa", s),
            ).expect("prefix must be base32")
        };

        let mut last_byte_idx = if s.is_empty() { 0 } else { 5 * s.len() / 8 };
        let n_bits = (5 * s.len()) % 8;
        let last_byte_mask = if n_bits > 0 {
            ((1 << n_bits) - 1) << (8 - n_bits)
        } else {
            0
        };
        if last_byte_mask > 0 && !s.is_empty() {
            last_byte_idx += 1;
        }

        let gpu_byte_prefix = rustacuda::memory::DeviceBuffer::from_slice(&byte_prefix).unwrap();
        let out = [0u8; 32];
        let gpu_out = rustacuda::memory::DeviceBuffer::from_slice(&out).unwrap();
        let success = false;
        let gpu_success = rustacuda::memory::DeviceBox::new(&success).unwrap();

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

impl eframe::App for VanityApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for new results
        if let Some(ref receiver) = self.result_receiver {
            while let Ok(result) = receiver.try_recv() {
                self.shared.results.lock().unwrap().push(result);
            }
        }

        let is_running = self.shared.running.load(Ordering::Relaxed);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Tor v3 Vanity Address Generator");
            ui.add_space(8.0);

            // Pattern input section
            ui.group(|ui| {
                ui.label("Patterns (valid: a-z, 2-7):");
                ui.add_space(4.0);

                let mut to_remove: Option<usize> = None;
                for (i, entry) in self.patterns.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [200.0, 20.0],
                            egui::TextEdit::singleline(&mut entry.pattern)
                                .hint_text("Enter pattern...")
                        );

                        egui::ComboBox::from_id_source(format!("pos_{}", i))
                            .width(80.0)
                            .selected_text(entry.position.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut entry.position, Position::Prefix, "Prefix");
                                ui.selectable_value(&mut entry.position, Position::Suffix, "Suffix");
                                ui.selectable_value(&mut entry.position, Position::Anywhere, "Anywhere");
                            });

                        if self.patterns.len() > 1 {
                            if ui.button("✕").clicked() {
                                to_remove = Some(i);
                            }
                        }
                    });
                }

                if let Some(i) = to_remove {
                    self.patterns.remove(i);
                }

                ui.add_space(4.0);
                if ui.button("+ Add Pattern").clicked() && self.patterns.len() < 10 {
                    self.patterns.push(PatternEntry::default());
                }
            });

            ui.add_space(8.0);

            // Output directory
            ui.horizontal(|ui| {
                ui.label("Output directory:");
                ui.add_sized(
                    [250.0, 20.0],
                    egui::TextEdit::singleline(&mut self.output_dir)
                );
                if ui.button("Browse...").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.output_dir = path.to_string_lossy().to_string();
                    }
                }
            });

            ui.add_space(8.0);

            // Time estimate
            if let Ok(patterns) = self.validate_patterns() {
                let estimate = VanityApp::estimate_time(&patterns);
                ui.label(format!("Estimated time: {}", estimate));
            }

            ui.add_space(8.0);

            // Error message
            if let Some(ref error) = self.error_message {
                ui.colored_label(egui::Color32::RED, error);
                ui.add_space(4.0);
            }

            // Status and progress
            ui.group(|ui| {
                let status = self.shared.status.lock().unwrap().clone();
                ui.label(format!("Status: {}", status));

                let tries = self.shared.tries.load(Ordering::Relaxed);
                let rate = *self.shared.rate.lock().unwrap();

                ui.label(format!("Keys tried: {}", format_number(tries)));
                ui.label(format!("Hash rate: {}/sec", format_number(rate as u64)));

                if let Some(start) = self.start_time {
                    if is_running {
                        let elapsed = start.elapsed();
                        ui.label(format!("Elapsed: {}:{:02}:{:02}",
                            elapsed.as_secs() / 3600,
                            (elapsed.as_secs() % 3600) / 60,
                            elapsed.as_secs() % 60
                        ));
                    }
                }
            });

            ui.add_space(8.0);

            // Control buttons
            ui.horizontal(|ui| {
                ui.set_enabled(!is_running);
                if ui.button("▶ Start").clicked() {
                    match self.validate_patterns() {
                        Ok(patterns) => self.start_generation(patterns),
                        Err(e) => self.error_message = Some(e),
                    }
                }

                ui.set_enabled(is_running);
                if ui.button("■ Stop").clicked() {
                    self.stop_generation();
                }
            });

            ui.add_space(8.0);

            // Results
            ui.group(|ui| {
                ui.label("Found addresses:");
                ui.add_space(4.0);

                let results = self.shared.results.lock().unwrap();
                if results.is_empty() {
                    ui.label("No results yet...");
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for result in results.iter().rev() {
                                ui.horizontal(|ui| {
                                    ui.label(&result.timestamp);
                                    ui.label(&result.address);
                                    if ui.button("Copy").clicked() {
                                        ui.output_mut(|o| {
                                            o.copied_text = result.address.clone();
                                        });
                                    }
                                });
                            }
                        });
                }
            });
        });

        // Request continuous updates while running
        if is_running {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

/// Format large numbers with commas
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([500.0, 650.0])
            .with_min_inner_size([400.0, 500.0])
            .with_title("Tor v3 Vanity Generator"),
        ..Default::default()
    };

    eframe::run_native(
        "Tor v3 Vanity Generator",
        options,
        Box::new(|_cc| Ok(Box::new(VanityApp::default()))),
    )
}
