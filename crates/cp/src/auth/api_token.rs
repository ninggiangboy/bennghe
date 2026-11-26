// SPDX-License-Identifier: AGPL-3.0-only
//! Personal API tokens for CI. Only the SHA-256 is stored; the plaintext is shown once.
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::util::{random_base62, sha256};

pub const PREFIX: &str = "bnp_";
pub const MAX_LIFETIME_DAYS: i64 = 365;

pub struct Generated {
    pub plaintext: String,
    pub hash: Vec<u8>,
    /// Last characters, enough to recognise a token in a list without revealing it.
    pub hint: String,
}

pub fn generate() -> Generated {
    let plaintext = format!("{PREFIX}{}", random_base62(43));
    let hint = plaintext[plaintext.len() - 4..].to_owned();
    Generated { hash: sha256(plaintext.as_bytes()), plaintext, hint }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub project_ids: Vec<Uuid>,
}

/// Finds a usable token: not revoked, not expired, and its user still enabled.
pub async fn authenticate(pool: &PgPool, presented: &str) -> Result<Option<Token>, sqlx::Error> {
    if !presented.starts_with(PREFIX) {
        return Ok(None);
    }
    let row = sqlx::query!(
        r#"select t.id, t.user_id, t.role, t.project_ids, t.last_used_at as "last_used_at: DateTime<Utc>"
           from api_tokens t join users u on u.id = t.user_id
          where t.token_hash = $1 and t.revoked_at is null and t.expires_at > now() and u.disabled_at is null"#,
        sha256(presented.as_bytes())
    )
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let stale = row.last_used_at.is_none_or(|t: DateTime<Utc>| Utc::now() - t > chrono::Duration::minutes(1));
    if stale {
        sqlx::query!("update api_tokens set last_used_at = now() where id = $1", row.id).execute(pool).await?;
    }
    Ok(Some(Token { id: row.id, user_id: row.user_id, role: row.role, project_ids: row.project_ids }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_have_the_scanning_prefix_and_a_matching_hash() {
        let t = generate();
        assert!(t.plaintext.starts_with("bnp_"));
        assert_eq!(t.plaintext.len(), 4 + 43);
        assert_eq!(t.hash, sha256(t.plaintext.as_bytes()));
        assert!(t.plaintext.ends_with(&t.hint));
    }
}
