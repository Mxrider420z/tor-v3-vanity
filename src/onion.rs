//! Tor v3 onion address generation utilities

use sha3::{Digest, Sha3_256};

/// Convert an Ed25519 public key to a Tor v3 onion address
///
/// The onion address format is: base32(pubkey || checksum || version).onion
/// where checksum = SHA3-256(".onion checksum" || pubkey || version)[0..2]
/// and version = 0x03
pub fn pubkey_to_onion(pubkey: &[u8; 32]) -> String {
    let mut hasher = Sha3_256::new();
    hasher.update(b".onion checksum");
    hasher.update(pubkey);
    hasher.update(&[3u8]);

    let mut onion = [0u8; 35];
    onion[..32].copy_from_slice(pubkey);
    onion[32..34].copy_from_slice(&hasher.finalize()[..2]);
    onion[34] = 3;

    format!(
        "{}.onion",
        base32::encode(base32::Alphabet::Rfc4648Lower { padding: false }, &onion)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_onion_format() {
        let pubkey = [0u8; 32];
        let onion = pubkey_to_onion(&pubkey);
        assert!(onion.ends_with(".onion"));
        assert_eq!(onion.len(), 62 + 6); // 56 base32 chars + ".onion"
    }
}
