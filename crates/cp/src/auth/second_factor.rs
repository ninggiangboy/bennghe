// SPDX-License-Identifier: AGPL-3.0-only
//! Checking a TOTP code or a recovery code for a user.
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::{recovery, totp};
use crate::secrets::Secrets;

pub fn aad(user_id: Uuid) -> String {
    format!("user_totp:{user_id}")
}

/// Accepts a current TOTP code (each time step works once) or an unused recovery code.
pub async fn check(pool: &PgPool, secrets: &Secrets, user_id: Uuid, code: &str) -> anyhow::Result<bool> {
    let row = sqlx::query!(
        "select secret_ciphertext, key_id, last_used_step from user_totp where user_id = $1 and confirmed_at is not null",
        user_id
    )
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(false) };
    let secret = secrets.open(&row.key_id, &aad(user_id), &row.secret_ciphertext)?;
    let now = u64::try_from(Utc::now().timestamp()).unwrap_or(0);
    if let Some(step) = totp::verify(&secret, code, now, u64::try_from(row.last_used_step).unwrap_or(0)) {
        // The conditional update is what makes a code single use even for concurrent requests.
        let step = i64::try_from(step)?;
        let won = sqlx::query!("update user_totp set last_used_step = $1 where user_id = $2 and last_used_step < $1", step, user_id)
            .execute(pool)
            .await?;
        return Ok(won.rows_affected() == 1);
    }
    let mut tx = pool.begin().await?;
    let ok = recovery::consume(&mut tx, user_id, code).await?;
    tx.commit().await?;
    Ok(ok)
}
