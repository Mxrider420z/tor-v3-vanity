//! Tor V3 Vanity Address Generator Library
//!
//! This library provides functionality to generate Tor v3 vanity addresses
//! with both CUDA GPU acceleration and CPU fallback support.

pub mod backend;
pub mod onion;

pub use backend::{
    select_backend, select_backend_with_mode, select_backend_with_config,
    Backend, BackendInfo, BackendMode, GeneratorError, FoundKey, Progress, format_speed,
};
pub use onion::pubkey_to_onion;

/// File prefix for Tor ed25519 secret key files
pub const FILE_PREFIX: &[u8] = b"== ed25519v1-secret: type0 ==\0\0\0";
