// SPDX-License-Identifier: AGPL-3.0-only
//! First-run setup: a one-time token printed on the host proves the person installing is the one
//! creating the owner account. Only a hash of the token is stored.
use sqlx::{PgConnection, PgPool};

use crate::util::{random_base62, sha256};

pub async fn needs_setup(pool: &PgPool) -> Result<bool, sqlx::Error> {
    Ok(!sqlx::query_scalar!(r#"select exists(select 1 from users) as "exists!""#).fetch_one(pool).await?)
}

/// Creates a new setup token and returns its plaintext. Older unused tokens stay valid until used.
pub async fn create_token(pool: &PgPool) -> Result<String, sqlx::Error> {
    let token = format!("bnsetup_{}", random_base62(32));
    sqlx::query!("insert into setup_tokens (token_hash) values ($1)", sha256(token.as_bytes())).execute(pool).await?;
    Ok(token)
}

/// Spends the token. Returns false when it is unknown or already used, so two simultaneous
/// setup requests cannot both create an owner.
pub async fn consume(tx: &mut PgConnection, presented: &str) -> Result<bool, sqlx::Error> {
    let spent = sqlx::query!(
        "update setup_tokens set used_at = now() where token_hash = $1 and used_at is null",
        sha256(presented.trim().as_bytes())
    )
    .execute(tx)
    .await?;
    Ok(spent.rows_affected() == 1)
}
