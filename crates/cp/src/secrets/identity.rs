// SPDX-License-Identifier: AGPL-3.0-only
//! Control-plane identity: instance id, restore epoch and the Ed25519 key that signs desired state.
use std::{fs, path::Path};

use anyhow::Context;
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::SigningKey;
use sqlx::PgPool;
use uuid::Uuid;

use super::{check_private, write_private};
use crate::util::random_bytes;

pub struct SigningKeyFile;

impl SigningKeyFile {
    /// Writes a new 32-byte Ed25519 seed with mode 0600. Never overwrites.
    pub fn generate(path: &Path) -> anyhow::Result<()> {
        write_private(path, format!("{}\n", STANDARD.encode(random_bytes::<32>())).as_bytes())
    }

    pub fn load(path: &Path) -> anyhow::Result<SigningKey> {
        check_private(path)?;
        let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let seed: [u8; 32] = STANDARD
            .decode(text.trim())
            .context("signing key is not valid base64")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("signing key must be 32 bytes"))?;
        Ok(SigningKey::from_bytes(&seed))
    }
}

pub struct Identity {
    pub instance_id: Uuid,
    /// Changes on every restore from backup, so agents can tell a rewound control plane apart.
    pub epoch: Uuid,
    pub signing_key: SigningKey,
}

impl Identity {
    /// Reads the identity from the database, creating it on first start. The public key stored in
    /// the database must match the key file, otherwise the wrong file was mounted.
    pub async fn ensure(pool: &PgPool, signing_key_path: &Path) -> anyhow::Result<Self> {
        let signing_key = SigningKeyFile::load(signing_key_path)?;
        let public = STANDARD.encode(signing_key.verifying_key().to_bytes());
        let instance_id = get_or_create(pool, "cp_instance_id", &Uuid::new_v4().to_string()).await?;
        let epoch = get_or_create(pool, "cp_epoch", &Uuid::new_v4().to_string()).await?;
        let stored_public = get_or_create(pool, "cp_public_key", &public).await?;
        if stored_public != public {
            anyhow::bail!("signing.key does not match the public key stored in the database; restore the original signing.key");
        }
        Ok(Self { instance_id: instance_id.parse()?, epoch: epoch.parse()?, signing_key })
    }
}

async fn get_or_create(pool: &PgPool, key: &str, default: &str) -> anyhow::Result<String> {
    let value = serde_json::Value::String(default.to_owned());
    sqlx::query!("insert into settings (key, value) values ($1, $2) on conflict (key) do nothing", key, value).execute(pool).await?;
    let stored = sqlx::query_scalar!("select value from settings where key = $1", key).fetch_one(pool).await?;
    stored.as_str().map(str::to_owned).with_context(|| format!("setting `{key}` is not a string"))
}
