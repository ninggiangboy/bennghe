// SPDX-License-Identifier: AGPL-3.0-only
//! Small helpers shared by several modules: randomness, hashing and constant-time compare.
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Fills a fixed-size array from the operating system's CSPRNG.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("operating system random source is unavailable");
    buf
}

/// Uniformly random base62 string of `len` characters (rejection sampling avoids modulo bias).
pub fn random_base62(len: usize) -> String {
    let mut out = String::with_capacity(len);
    while out.len() < len {
        for b in random_bytes::<64>() {
            // 248 = 62 * 4, the largest multiple of 62 that fits in a byte.
            if b < 248 && out.len() < len {
                out.push(BASE62[(b % 62) as usize] as char);
            }
        }
    }
    out
}

pub fn sha256(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base62_has_requested_length_and_alphabet() {
        let s = random_base62(100);
        assert_eq!(s.len(), 100);
        assert!(s.bytes().all(|b| BASE62.contains(&b)));
        assert_ne!(random_base62(32), random_base62(32));
    }
}
