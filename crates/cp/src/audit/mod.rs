// SPDX-License-Identifier: AGPL-3.0-only
//! Audit trail. Successful changes are recorded in the same transaction as the change;
//! refused requests are recorded in their own transaction because the request's one rolls back.
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct Actor {
    pub user_id: Option<Uuid>,
    pub token_id: Option<Uuid>,
    pub ip: Option<String>,
}

pub struct Entry<'a> {
    pub action: &'a str,
    pub scope: &'a str,
    pub scope_id: Option<Uuid>,
    pub resource_type: Option<&'a str>,
    pub resource_id: Option<String>,
    pub metadata: Value,
}

impl<'a> Entry<'a> {
    pub fn new(action: &'a str, scope: &'a str) -> Self {
        Self { action, scope, scope_id: None, resource_type: None, resource_id: None, metadata: json!({}) }
    }
    pub fn scope_id(mut self, id: Option<Uuid>) -> Self {
        self.scope_id = id;
        self
    }
    pub fn resource(mut self, kind: &'a str, id: impl ToString) -> Self {
        self.resource_type = Some(kind);
        self.resource_id = Some(id.to_string());
        self
    }
    pub fn meta(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Records a successful operation (or a sensitive read) in the caller's transaction.
pub async fn record(tx: &mut PgConnection, actor: &Actor, e: &Entry<'_>) -> Result<(), sqlx::Error> {
    insert(tx, actor, e, "success", None).await
}

/// Records a refused request in its own transaction. A failure here is logged, never propagated,
/// so an audit outage cannot turn a refusal into an error that hides the refusal.
pub async fn record_denied(pool: &PgPool, actor: &Actor, e: &Entry<'_>, reason: &str) {
    let result = async {
        let mut conn = pool.acquire().await?;
        insert(&mut conn, actor, e, "denied", Some(reason)).await
    }
    .await;
    if let Err(err) = result {
        tracing::error!(error = %err, action = e.action, "failed to record denied request");
    }
}

async fn insert(conn: &mut PgConnection, actor: &Actor, e: &Entry<'_>, outcome: &str, reason: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "insert into audit_events
           (actor_user_id, actor_token_id, actor_ip, action, scope, scope_id, resource_type, resource_id, outcome, reason, metadata)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        actor.user_id,
        actor.token_id,
        actor.ip,
        e.action,
        e.scope,
        e.scope_id,
        e.resource_type,
        e.resource_id,
        outcome,
        reason,
        e.metadata
    )
    .execute(conn)
    .await?;
    Ok(())
}
