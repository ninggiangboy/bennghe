// SPDX-License-Identifier: AGPL-3.0-only
//! Encryption of secrets at rest with AES-256-GCM.
//!
//! The associated data binds a ciphertext to the table and row it belongs to, so a value copied
//! to another row fails to decrypt. The key file holds one `key_id:base64(key)` per line; the
//! first line is the active key and later lines are older keys kept for decrypting.
mod identity;

use std::{collections::HashMap, fs, io::Write, os::unix::fs::OpenOptionsExt, path::Path};

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use anyhow::{Context, bail};
use base64::{Engine, engine::general_purpose::STANDARD};

pub use identity::{Identity, SigningKeyFile};

use crate::util::random_bytes;

const NONCE_LEN: usize = 12;
const CANARY_AAD: &str = "encryption_keys:canary";
const CANARY: &[u8] = b"bennghe-master-key-canary";

/// Ciphertext together with the id of the key that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub key_id: String,
    /// `nonce || ciphertext || tag`
    pub data: Vec<u8>,
}

pub struct Secrets {
    active: String,
    keys: HashMap<String, Aes256Gcm>,
}

impl Secrets {
    /// Loads the key file, refusing one that other users on the host can read.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        check_private(path)?;
        let text = fs::read_to_string(path).with_context(|| format!("reading master key {}", path.display()))?;
        Self::parse(&text)
    }

    fn parse(text: &str) -> anyhow::Result<Self> {
        let mut active = None;
        let mut keys = HashMap::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let (id, b64) = line.split_once(':').context("master key line must be `key_id:base64`")?;
            let raw = STANDARD.decode(b64.trim()).context("master key is not valid base64")?;
            let cipher = Aes256Gcm::new_from_slice(&raw).map_err(|_| anyhow::anyhow!("master key must be 32 bytes"))?;
            active.get_or_insert_with(|| id.to_owned());
            keys.insert(id.to_owned(), cipher);
        }
        let active = active.context("master key file has no keys")?;
        Ok(Self { active, keys })
    }

    /// Writes a fresh single-key file with mode 0600. Never overwrites an existing file.
    pub fn generate_file(path: &Path) -> anyhow::Result<()> {
        let key = random_bytes::<32>();
        write_private(path, format!("k1:{}\n", STANDARD.encode(key)).as_bytes())
    }

    pub fn active_key_id(&self) -> &str {
        &self.active
    }

    pub fn seal(&self, aad: &str, plaintext: &[u8]) -> anyhow::Result<Sealed> {
        let cipher = &self.keys[&self.active];
        let nonce_bytes = random_bytes::<NONCE_LEN>();
        let ciphertext = cipher
            .encrypt(&Nonce::try_from(&nonce_bytes[..]).expect("12-byte nonce"), Payload { msg: plaintext, aad: aad.as_bytes() })
            .map_err(|_| anyhow::anyhow!("encryption failed"))?;
        let mut data = nonce_bytes.to_vec();
        data.extend_from_slice(&ciphertext);
        Ok(Sealed { key_id: self.active.clone(), data })
    }

    pub fn open(&self, key_id: &str, aad: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
        let cipher = self.keys.get(key_id).with_context(|| format!("unknown key id `{key_id}`"))?;
        if data.len() < NONCE_LEN {
            bail!("ciphertext is too short");
        }
        let (nonce, ciphertext) = data.split_at(NONCE_LEN);
        cipher
            .decrypt(&Nonce::try_from(nonce).expect("12-byte nonce"), Payload { msg: ciphertext, aad: aad.as_bytes() })
            .map_err(|_| anyhow::anyhow!("decryption failed: wrong key, wrong row or corrupted data"))
    }

    /// Fails fast when the key file does not match the database it is used with.
    pub async fn verify_against(&self, pool: &sqlx::PgPool) -> anyhow::Result<()> {
        for (key_id, _) in self.keys.iter().filter(|(id, _)| id.as_str() == self.active) {
            let row = sqlx::query!("select canary_ciphertext from encryption_keys where key_id = $1", key_id).fetch_optional(pool).await?;
            match row {
                Some(r) => {
                    let plain = self
                        .open(key_id, CANARY_AAD, &r.canary_ciphertext)
                        .context("the master key does not match this database; restore the original master.key")?;
                    if plain != CANARY {
                        bail!("master key canary mismatch");
                    }
                }
                None => {
                    let sealed = self.seal(CANARY_AAD, CANARY)?;
                    sqlx::query!(
                        "insert into encryption_keys (key_id, canary_ciphertext) values ($1, $2) on conflict do nothing",
                        sealed.key_id,
                        sealed.data
                    )
                    .execute(pool)
                    .await?;
                }
            }
        }
        // A database that remembers keys this file lacks cannot decrypt its own rows.
        let known: Vec<String> = sqlx::query_scalar!("select key_id from encryption_keys").fetch_all(pool).await?;
        for id in known {
            if !self.keys.contains_key(&id) {
                bail!("the database has data encrypted with key `{id}` which is missing from the master key file");
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self::parse(&format!("k1:{}\n", STANDARD.encode([7u8; 32]))).unwrap()
    }
}

/// Writes a secret file that only the current user can read; fails if it already exists.
pub(crate) fn write_private(path: &Path, content: &[u8]) -> anyhow::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("creating {} (it must not already exist)", path.display()))?;
    file.write_all(content)?;
    Ok(())
}

pub(crate) fn check_private(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path).with_context(|| format!("reading {}", path.display()))?.permissions().mode();
    if mode & 0o077 != 0 {
        bail!("{} has mode {:o}; it must not be readable by group or others (chmod 600)", path.display(), mode & 0o777);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_aad_binding() {
        let s = Secrets::for_tests();
        let sealed = s.seal("user_totp:abc", b"hunter2").unwrap();
        assert_eq!(s.open(&sealed.key_id, "user_totp:abc", &sealed.data).unwrap(), b"hunter2");
        assert!(s.open(&sealed.key_id, "user_totp:other", &sealed.data).is_err(), "AAD must bind the row");
    }

    #[test]
    fn nonces_are_not_reused() {
        let s = Secrets::for_tests();
        assert_ne!(s.seal("a", b"x").unwrap().data, s.seal("a", b"x").unwrap().data);
    }

    #[test]
    fn tampering_is_detected() {
        let s = Secrets::for_tests();
        let mut sealed = s.seal("a", b"secret").unwrap();
        *sealed.data.last_mut().unwrap() ^= 1;
        assert!(s.open(&sealed.key_id, "a", &sealed.data).is_err());
    }

    #[test]
    fn rotation_keeps_old_keys_readable() {
        let old = Secrets::parse(&format!("k1:{}", STANDARD.encode([1u8; 32]))).unwrap();
        let sealed = old.seal("a", b"v").unwrap();
        let rotated = Secrets::parse(&format!("k2:{}\nk1:{}", STANDARD.encode([2u8; 32]), STANDARD.encode([1u8; 32]))).unwrap();
        assert_eq!(rotated.active_key_id(), "k2");
        assert_eq!(rotated.open("k1", "a", &sealed.data).unwrap(), b"v");
        assert_eq!(rotated.seal("a", b"v").unwrap().key_id, "k2");
    }

    #[test]
    fn group_readable_key_file_is_refused() {
        let dir = tempdir();
        let path = dir.join("master.key");
        Secrets::generate_file(&path).unwrap();
        assert!(Secrets::load(&path).is_ok());
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Secrets::load(&path).is_err());
        assert!(Secrets::generate_file(&path).is_err(), "must never overwrite an existing key");
    }

    pub(crate) fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bn-test-{}", crate::util::random_base62(10)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
