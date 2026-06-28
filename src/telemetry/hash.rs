//! Domain-separated, salted, truncated BLAKE3 hashes for AL identifiers.
//!
//! See spec §5 "Hashing rules". 128-bit (32-char hex) for queryable identifiers,
//! 64-bit (16-char hex) for `install_id` and `workspace_id`.

use blake3::Hasher;

/// Domain tags for hash inputs. Prevents cross-field collision.
pub const DOMAIN_OBJECT: &[u8] = b"object:";
pub const DOMAIN_PROCEDURE: &[u8] = b"procedure:";
pub const DOMAIN_APP_ID: &[u8] = b"app_id:";
pub const DOMAIN_FILE: &[u8] = b"file:";
pub const DOMAIN_WORKSPACE: &[u8] = b"workspace:";
#[allow(dead_code)] // domain-separation tag for future node-kind hashing
pub const DOMAIN_NODE_KIND: &[u8] = b"node_kind:";

const MAX_INPUT_BYTES: usize = 4096;

/// 32-byte salt (the local installation-id).
pub type Salt = [u8; 32];

/// Hash an AL identifier with domain separation. Returns 32-char lowercase hex
/// (128 bits of digest).
pub fn hash_identifier(salt: &Salt, domain: &[u8], input: &str) -> String {
    let normalized = input.to_lowercase();
    let bytes = normalized.as_bytes();
    let truncated = &bytes[..bytes.len().min(MAX_INPUT_BYTES)];

    let mut h = Hasher::new_keyed(salt);
    h.update(domain);
    h.update(truncated);
    let digest = h.finalize();
    hex_lower_truncated(digest.as_bytes(), 16)
}

/// 16-char hex form for `install_id`/`workspace_id` (64 bits).
pub fn hash_short(salt: &Salt, domain: &[u8], input: &[u8]) -> String {
    let mut h = Hasher::new_keyed(salt);
    h.update(domain);
    let truncated = &input[..input.len().min(MAX_INPUT_BYTES)];
    h.update(truncated);
    let digest = h.finalize();
    hex_lower_truncated(digest.as_bytes(), 8)
}

/// Compute the public `install_id` from the salt itself: blake3(salt)[..8] → 16 hex.
pub fn install_id_from_salt(salt: &Salt) -> String {
    let digest = blake3::hash(salt);
    hex_lower_truncated(digest.as_bytes(), 8)
}

fn hex_lower_truncated(bytes: &[u8], n: usize) -> String {
    let mut s = String::with_capacity(n * 2);
    for &b in &bytes[..n] {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn salt() -> Salt {
        [0x42; 32]
    }

    #[test]
    fn hash_identifier_is_deterministic() {
        let s = salt();
        let a = hash_identifier(&s, DOMAIN_PROCEDURE, "PostInvoice");
        let b = hash_identifier(&s, DOMAIN_PROCEDURE, "PostInvoice");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_identifier_is_case_insensitive() {
        let s = salt();
        let a = hash_identifier(&s, DOMAIN_PROCEDURE, "PostInvoice");
        let b = hash_identifier(&s, DOMAIN_PROCEDURE, "postinvoice");
        let c = hash_identifier(&s, DOMAIN_PROCEDURE, "POSTINVOICE");
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn different_salts_produce_different_hashes() {
        let s1 = [0x42; 32];
        let s2 = [0x43; 32];
        let a = hash_identifier(&s1, DOMAIN_PROCEDURE, "PostInvoice");
        let b = hash_identifier(&s2, DOMAIN_PROCEDURE, "PostInvoice");
        assert_ne!(a, b);
    }

    #[test]
    fn different_domains_produce_different_hashes() {
        let s = salt();
        let as_object = hash_identifier(&s, DOMAIN_OBJECT, "Customer");
        let as_procedure = hash_identifier(&s, DOMAIN_PROCEDURE, "Customer");
        assert_ne!(as_object, as_procedure);
    }

    #[test]
    fn install_id_is_16_chars() {
        let id = install_id_from_salt(&salt());
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_short_is_16_chars() {
        let s = salt();
        let id = hash_short(&s, DOMAIN_WORKSPACE, b"/some/path");
        assert_eq!(id.len(), 16);
    }

    #[test]
    fn extreme_input_is_truncated_not_panicked() {
        let s = salt();
        let huge = "a".repeat(10 * 1024 * 1024); // 10MB
        let _ = hash_identifier(&s, DOMAIN_PROCEDURE, &huge);
        // No panic, no OOM, returns within reasonable time.
    }
}
