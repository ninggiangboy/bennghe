// SPDX-License-Identifier: AGPL-3.0-only
//! Browser sessions. The cookie carries a random token; only its SHA-256 is stored.
use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::util::{random_base62, sha256};

pub const ACTIVE_TTL: Duration = Duration::days(7);
/// Time allowed between a correct password and the second-factor code.
pub const PENDING_TTL: Duration = Duration::minutes(5);
/// How recent a password or code check must be for sensitive operations.
pub const REAUTH_WINDOW: Duration = Duration::minutes(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    PendingSecondFactor,
    Active,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::PendingSecondFactor => "pending_2fa",
            Stage::Active => "active",
        }
    }
    fn parse(s: &str) -> Self {
        if s == "active" { Stage::Active } else { Stage::PendingSecondFactor }
    }
}

pub struct Created {
    pub token: String,
    pub csrf_token: String,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub stage: Stage,
    pub csrf_token: String,
    pub last_verified_at: DateTime<Utc>,
}

fn new_token() -> String {
    random_base62(43)
}

pub async fn create(pool: &PgPool, user_id: Uuid, stage: Stage, user_agent: Option<&str>) -> Result<Created, sqlx::Error> {
    let token = new_token();
    let csrf_token = new_token();
    let ttl = if stage == Stage::Active { ACTIVE_TTL } else { PENDING_TTL };
    sqlx::query!(
        "insert into sessions (token_hash, user_id, csrf_token, stage, expires_at, last_verified_at, user_agent)
         values ($1, $2, $3, $4, now() + $5::interval, now(), $6)",
        sha256(token.as_bytes()),
        user_id,
        csrf_token,
        stage.as_str(),
        sqlx::postgres::types::PgInterval::try_from(ttl.to_std().expect("positive ttl")).expect("interval"),
        user_agent,
    )
    .execute(pool)
    .await?;
    Ok(Created { token, csrf_token })
}

/// Finds a live session for the cookie value; expired sessions and disabled users never match.
pub async fn lookup(pool: &PgPool, token: &str) -> Result<Option<Session>, sqlx::Error> {
    let row = sqlx::query!(
        r#"select s.id, s.user_id, s.stage, s.csrf_token, s.last_verified_at as "last_verified_at: DateTime<Utc>"
           from sessions s join users u on u.id = s.user_id
          where s.token_hash = $1 and s.expires_at > now() and u.disabled_at is null"#,
        sha256(token.as_bytes())
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| Session {
        id: r.id,
        user_id: r.user_id,
        stage: Stage::parse(&r.stage),
        csrf_token: r.csrf_token,
        last_verified_at: r.last_verified_at,
    }))
}

/// Turns a pending session into an active one under a fresh token and CSRF token, so a token
/// captured before the second factor is worthless afterwards.
pub async fn activate(pool: &PgPool, session_id: Uuid) -> Result<Created, sqlx::Error> {
    let created = Created { token: new_token(), csrf_token: new_token() };
    sqlx::query!(
        "update sessions set token_hash = $2, csrf_token = $3, stage = 'active', last_verified_at = now(),
                expires_at = now() + interval '7 days'
          where id = $1",
        session_id,
        sha256(created.token.as_bytes()),
        created.csrf_token
    )
    .execute(pool)
    .await?;
    Ok(created)
}

pub async fn mark_verified(pool: &PgPool, session_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("update sessions set last_verified_at = now() where id = $1", session_id).execute(pool).await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, session_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("delete from sessions where id = $1", session_id).execute(pool).await?;
    Ok(())
}

pub async fn delete_all_for_user(pool: &PgPool, user_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("delete from sessions where user_id = $1", user_id).execute(pool).await?;
    Ok(())
}

pub async fn purge_expired(pool: &PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query!("delete from sessions where expires_at < now()").execute(pool).await?.rows_affected())
}
