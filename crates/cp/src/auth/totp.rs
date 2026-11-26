// SPDX-License-Identifier: AGPL-3.0-only
//! TOTP (RFC 6238, HMAC-SHA1, 6 digits, 30 second steps) as used by authenticator apps.
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

use crate::util::{ct_eq, random_bytes};

pub const STEP_SECONDS: u64 = 30;
const DIGITS: u32 = 6;

pub fn generate_secret() -> [u8; 20] {
    random_bytes::<20>()
}

pub fn encode_secret(secret: &[u8]) -> String {
    base32::encode(base32::Alphabet::Rfc4648 { padding: false }, secret)
}

pub fn otpauth_uri(issuer: &str, account: &str, secret: &[u8]) -> String {
    let enc = |s: &str| {
        s.bytes()
            .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
            .collect::<String>()
    };
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECONDS}",
        enc(issuer),
        enc(account),
        encode_secret(secret),
        enc(issuer)
    )
}

pub fn step_at(unix_seconds: u64) -> u64 {
    unix_seconds / STEP_SECONDS
}

/// The code an authenticator shows for a given step (HOTP, RFC 4226).
pub fn code_at(secret: &[u8], step: u64) -> String {
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let bin = u32::from_be_bytes([digest[offset] & 0x7f, digest[offset + 1], digest[offset + 2], digest[offset + 3]]);
    format!("{:0width$}", bin % 10u32.pow(DIGITS), width = DIGITS as usize)
}

/// Checks a submitted code against the current step and one step either side (clock drift).
/// Returns the matching step, or `None` when the code is wrong or its step was already used.
/// The caller must persist the returned step so the same code cannot be replayed.
pub fn verify(secret: &[u8], submitted: &str, now_unix: u64, last_used_step: u64) -> Option<u64> {
    let submitted = submitted.trim();
    if submitted.len() != DIGITS as usize || !submitted.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let now = step_at(now_unix);
    let mut matched = None;
    // Every candidate is compared so timing does not reveal which step matched.
    for step in now.saturating_sub(1)..=now + 1 {
        if ct_eq(code_at(secret, step).as_bytes(), submitted.as_bytes()) && step > last_used_step {
            matched = Some(step);
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238 appendix B: secret "12345678901234567890", T = 59 s gives 94287082 (8 digits).
    #[test]
    fn matches_rfc_6238_vector() {
        let secret = b"12345678901234567890";
        assert_eq!(code_at(secret, step_at(59)), "287082");
        assert_eq!(code_at(secret, step_at(1111111109)), "081804");
        assert_eq!(code_at(secret, step_at(2000000000)), "279037");
    }

    #[test]
    fn accepts_adjacent_steps_only() {
        let secret = b"12345678901234567890";
        let now = 1_700_000_000;
        let step = step_at(now);
        assert_eq!(verify(secret, &code_at(secret, step), now, 0), Some(step));
        assert_eq!(verify(secret, &code_at(secret, step - 1), now, 0), Some(step - 1));
        assert_eq!(verify(secret, &code_at(secret, step + 1), now, 0), Some(step + 1));
        assert_eq!(verify(secret, &code_at(secret, step - 2), now, 0), None);
    }

    #[test]
    fn a_used_step_cannot_be_replayed() {
        let secret = b"12345678901234567890";
        let now = 1_700_000_000;
        let step = step_at(now);
        let code = code_at(secret, step);
        assert_eq!(verify(secret, &code, now, step - 1), Some(step));
        assert_eq!(verify(secret, &code, now, step), None);
    }

    #[test]
    fn malformed_codes_are_rejected() {
        let secret = b"12345678901234567890";
        for bad in ["", "12345", "1234567", "abcdef", "12 456"] {
            assert_eq!(verify(secret, bad, 1_700_000_000, 0), None);
        }
    }

    #[test]
    fn otpauth_uri_escapes_account() {
        let uri = otpauth_uri("Bennghe", "a b@example.com", b"12345678901234567890");
        assert!(uri.starts_with("otpauth://totp/Bennghe:a%20b%40example.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"));
    }
}
