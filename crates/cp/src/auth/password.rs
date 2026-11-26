// SPDX-License-Identifier: AGPL-3.0-only
//! Password hashing with Argon2id. Hashing is CPU-heavy, so it runs off the async threads.
use argon2::{Argon2, PasswordHasher, PasswordVerifier, password_hash::phc::PasswordHash};

pub const MIN_LENGTH: usize = 12;
pub const MAX_LENGTH: usize = 256;

pub fn validate_new(password: &str) -> Result<(), String> {
    let n = password.chars().count();
    if n < MIN_LENGTH {
        return Err(format!("password must be at least {MIN_LENGTH} characters"));
    }
    if password.len() > MAX_LENGTH {
        return Err(format!("password must be at most {MAX_LENGTH} bytes"));
    }
    Ok(())
}

pub async fn hash(password: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || hash_blocking(&password)).await?
}

pub fn hash_blocking(password: &str) -> anyhow::Result<String> {
    let hash: PasswordHash =
        Argon2::default().hash_password(password.as_bytes()).map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))?;
    Ok(hash.to_string())
}

pub async fn verify(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || verify_blocking(&password, &hash)).await.unwrap_or(false)
}

pub fn verify_blocking(password: &str, hash: &str) -> bool {
    Argon2::default().verify_password(password.as_bytes(), hash).is_ok()
}

/// A valid hash of a random password, verified against when the account does not exist so that
/// login time does not reveal which emails are registered.
pub fn dummy_hash() -> &'static str {
    use std::sync::OnceLock;
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| hash_blocking(&crate::util::random_base62(24)).expect("hash a random password"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_verifies_and_is_argon2id() {
        let h = hash_blocking("correct horse battery").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_blocking("correct horse battery", &h));
        assert!(!verify_blocking("wrong horse battery", &h));
    }

    #[test]
    fn short_passwords_are_rejected() {
        assert!(validate_new("short").is_err());
        assert!(validate_new("long enough password").is_ok());
    }
}
