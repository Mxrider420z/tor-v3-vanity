//! Tor V3 Vanity Address Generator CLI
//!
//! A high-performance vanity address generator with GPU acceleration and CPU fallback.

use clap::{Parser, ValueEnum};
use crossbeam_channel::unbounded;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tor_v3_vanity::backend::{select_backend_with_mode, BackendMode, Progress};

#[derive(Parser)]
#[command(name = "t3v")]
#[command(about = "Tor V3 vanity address generator with GPU acceleration")]
#[command(version)]
struct Cli {
    /// Desired prefixes (comma-separated)
    #[arg(required = true, value_delimiter = ',')]
    prefixes: Vec<String>,

    /// Output directory for generated keys
    #[arg(short, long, default_value = ".")]
    dst: PathBuf,

    /// Backend mode
    #[arg(short, long, value_enum, default_value = "auto")]
    mode: Mode,

    /// Number of CPU threads (only used in cpu and hybrid modes)
    #[arg(short = 't', long, default_value_t = num_cpus::get())]
    threads: usize,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Mode {
    /// Automatically select best backend (Hybrid > CUDA > CPU)
    Auto,
    /// CPU only
    Cpu,
    /// CUDA GPU only
    #[cfg(feature = "cuda")]
    Cuda,
    /// Hybrid CPU + GPU (maximum speed)
    #[cfg(feature = "cuda")]
    Hybrid,
}

impl From<Mode> for BackendMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Auto => BackendMode::Auto,
            Mode::Cpu => BackendMode::Cpu,
            #[cfg(feature = "cuda")]
            Mode::Cuda => BackendMode::Cuda,
            #[cfg(feature = "cuda")]
            Mode::Hybrid => BackendMode::Hybrid,
        }
    }
}

/// Pretty duration formatter
struct PrettyDur(chrono::Duration);

impl std::fmt::Display for PrettyDur {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.num_weeks() >= 52 {
            write!(f, "{} years, ", self.0.num_weeks() / 52)?;
        }
        if self.0.num_weeks() % 52 > 0 {
            write!(f, "{} weeks, ", self.0.num_weeks() % 52)?;
        }
        if self.0.num_days() % 7 > 0 {
            write!(f, "{} days, ", self.0.num_days() % 7)?;
        }
        if self.0.num_hours() % 24 > 0 {
            write!(f, "{} hours, ", self.0.num_hours() % 24)?;
        }
        if self.0.num_minutes() % 60 > 0 {
            write!(f, "{} minutes, ", self.0.num_minutes() % 60)?;
        }
        write!(f, "{} seconds", self.0.num_seconds() % 60)
    }
}

fn main() {
    let cli = Cli::parse();

    // Validate output directory
    if !cli.dst.is_dir() {
        eprintln!("Error: '{}' is not a directory", cli.dst.display());
        std::process::exit(1);
    }

    // Validate prefixes
    let max_len = cli.prefixes.iter().map(|p| p.len()).max().unwrap_or(0);
    for prefix in &cli.prefixes {
        if prefix.is_empty() {
            eprintln!("Error: Empty prefix not allowed");
            std::process::exit(1);
        }
        // Check if valid base32
        if base32::decode(
            base32::Alphabet::Rfc4648Lower { padding: false },
            &format!("{}aa", prefix),
        )
        .is_none()
        {
            eprintln!("Error: '{}' is not a valid base32 prefix", prefix);
            std::process::exit(1);
        }
    }

    println!("=== Tor V3 Vanity Generator ===");
    println!("Prefixes: {:?}", cli.prefixes);
    println!("Output: {}", cli.dst.display());
    println!("CPU threads: {}", cli.threads);
    println!();

    // Select backend
    let backend = select_backend_with_mode(cli.mode.into());
    let info = backend.info();

    println!();
    println!("Starting generation...");
    println!();

    // Set up channels
    let (progress_tx, progress_rx) = unbounded::<Progress>();
    let (result_tx, result_rx) = unbounded();
    let (stop_tx, stop_rx) = crossbeam_channel::bounded(1);

    // Handle Ctrl+C
    let stop_tx_clone = stop_tx.clone();
    ctrlc::set_handler(move || {
        eprintln!("\nStopping...");
        let _ = stop_tx_clone.send(());
    })
    .ok();

    // Clone values for threads
    let prefixes = cli.prefixes.clone();
    let dst = cli.dst.clone();

    // Spawn generation thread
    let gen_handle = std::thread::spawn(move || {
        backend.generate(prefixes, dst, progress_tx, result_tx, stop_rx)
    });

    // Progress display thread
    let start_time = Instant::now();
    let expected = 2_f64.powi(5 * max_len as i32);
    let mut last_log = Instant::now();
    let mut found_count = 0;
    let total_prefixes = cli.prefixes.len();

    loop {
        // Check for results
        while let Ok(result) = result_rx.try_recv() {
            found_count += 1;
            println!(
                "FOUND [{}/{}]: {} -> {}",
                found_count, total_prefixes, result.prefix, result.onion_address
            );
            println!("  Saved to: {}", result.key_path.display());
        }

        // Check for progress
        if let Ok(progress) = progress_rx.try_recv() {
            if last_log.elapsed() > Duration::from_secs(10) {
                let dur = progress.elapsed_secs;
                let dur_pretty = PrettyDur(
                    chrono::Duration::from_std(Duration::from_secs_f64(dur)).unwrap_or(chrono::Duration::zero()),
                );

                let progress_pct = progress.keys_checked as f64 / expected;
                let expected_dur = if progress_pct > 0.0 {
                    dur / progress_pct
                } else {
                    0.0
                };
                let expected_dur_pretty = PrettyDur(
                    chrono::Duration::from_std(Duration::from_secs_f64(expected_dur))
                        .unwrap_or(chrono::Duration::zero()),
                );

                println!();
                println!(
                    "Progress: {:.2e} / {:.2e} keys ({:.4}%)",
                    progress.keys_checked as f64,
                    expected,
                    progress_pct * 100.0
                );
                println!(
                    "Speed: {:.2} M keys/sec",
                    progress.keys_per_sec / 1_000_000.0
                );
                println!("Elapsed: {} / Est. total: {}", dur_pretty, expected_dur_pretty);
                println!("Found: {}/{} prefixes", found_count, total_prefixes);
                println!();

                last_log = Instant::now();
            }
        }

        // Check if generation is done
        if found_count >= total_prefixes {
            break;
        }

        // Small sleep to prevent busy loop
        std::thread::sleep(Duration::from_millis(50));

        // Check if generator thread has finished
        if gen_handle.is_finished() {
            break;
        }
    }

    // Wait for generator
    match gen_handle.join() {
        Ok(Ok(())) => {
            println!();
            println!("=== Complete! ===");
            println!("Found all {} prefixes in {}", total_prefixes,
                PrettyDur(chrono::Duration::from_std(start_time.elapsed()).unwrap_or(chrono::Duration::zero())));
        }
        Ok(Err(e)) => {
            eprintln!();
            eprintln!("Generation stopped: {}", e);
        }
        Err(_) => {
            eprintln!();
            eprintln!("Generation thread panicked");
        }
    }
}
