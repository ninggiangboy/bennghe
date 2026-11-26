// SPDX-License-Identifier: AGPL-3.0-only
//! Installation-wide administration: security settings, the audit log and user recovery.
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::extract::ApiJson;
use crate::{
    app::AppState,
    audit::{self, Entry},
    auth::session,
    error::{ApiError, ApiResult, ErrorBody},
    events::{self, Event, kind},
    httpserver::{Authn, authn},
};

#[derive(Serialize, Deserialize, ToSchema)]
pub struct SecuritySettings {
    /// Owners and admins must enable two-factor authentication before doing anything else.
    pub require_2fa_for_admins: bool,
}

#[utoipa::path(
    operation_id = "admin_security_get",
    get,
    path = "/api/v1/settings/security",
    tag = "settings",
    responses((status = 200, body = SecuritySettings)),
    extensions(("x-permission" = json!({"action": "settings.read", "scope": "global"}))),
)]
pub async fn security_get(State(app): State<AppState>) -> ApiResult<Json<SecuritySettings>> {
    Ok(Json(SecuritySettings { require_2fa_for_admins: authn::require_2fa_for_admins(&app.pool).await? }))
}

#[utoipa::path(
    operation_id = "admin_security_put",
    put,
    path = "/api/v1/settings/security",
    tag = "settings",
    request_body = SecuritySettings,
    responses((status = 200, body = SecuritySettings)),
    extensions(("x-permission" = json!({"action": "settings.update", "scope": "global"}))),
)]
pub async fn security_put(
    State(app): State<AppState>,
    Extension(me): Extension<Authn>,
    ApiJson(req): ApiJson<SecuritySettings>,
) -> ApiResult<Json<SecuritySettings>> {
    let mut tx = app.pool.begin().await?;
    sqlx::query!(
        "insert into settings (key, value) values ('require_2fa_for_admins', $1)
         on conflict (key) do update set value = excluded.value, updated_at = now()",
        json!(req.require_2fa_for_admins)
    )
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        &me.actor,
        &Entry::new("settings.update", "global").meta(json!({ "require_2fa_for_admins": req.require_2fa_for_admins })),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(req))
}

#[derive(Deserialize, IntoParams)]
pub struct AuditQuery {
    /// Page size, 1 to 200. Defaults to 50.
    pub limit: Option<i64>,
    /// Return events older than this id (from the previous page).
    pub before_id: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct AuditEvent {
    pub id: i64,
    pub occurred_at: DateTime<Utc>,
    pub actor_user_id: Option<Uuid>,
    pub actor_token_id: Option<Uuid>,
    pub actor_ip: Option<String>,
    pub action: String,
    pub scope: String,
    pub scope_id: Option<Uuid>,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    /// `success` or `denied`.
    pub outcome: String,
    pub reason: Option<String>,
    pub metadata: Value,
}

#[utoipa::path(
    operation_id = "admin_audit_list",
    get,
    path = "/api/v1/audit-events",
    tag = "audit",
    params(AuditQuery),
    responses((status = 200, description = "Newest first", body = Vec<AuditEvent>)),
    extensions(("x-permission" = json!({"action": "audit.read", "scope": "global"}))),
)]
pub async fn audit_list(State(app): State<AppState>, Query(q): Query<AuditQuery>) -> ApiResult<Json<Vec<AuditEvent>>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let rows = sqlx::query!(
        r#"select id, occurred_at as "occurred_at: DateTime<Utc>", actor_user_id, actor_token_id, actor_ip, action, scope, scope_id,
                  resource_type, resource_id, outcome, reason, metadata
             from audit_events where ($1::bigint is null or id < $1) order by id desc limit $2"#,
        q.before_id,
        limit
    )
    .fetch_all(&app.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| AuditEvent {
                id: r.id,
                occurred_at: r.occurred_at,
                actor_user_id: r.actor_user_id,
                actor_token_id: r.actor_token_id,
                actor_ip: r.actor_ip,
                action: r.action,
                scope: r.scope,
                scope_id: r.scope_id,
                resource_type: r.resource_type,
                resource_id: r.resource_id,
                outcome: r.outcome,
                reason: r.reason,
                metadata: r.metadata,
            })
            .collect(),
    ))
}

#[utoipa::path(
    operation_id = "admin_totp_reset",
    post,
    path = "/api/v1/users/{user_id}/totp/reset",
    tag = "users",
    params(("user_id" = Uuid, Path, description = "User whose second factor is removed")),
    responses((status = 204, description = "Reset"), (status = 404, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "user.totp_reset", "scope": "global", "reauth": true}))),
)]
pub async fn totp_reset(State(app): State<AppState>, Extension(me): Extension<Authn>, Path(user_id): Path<Uuid>) -> ApiResult<StatusCode> {
    let mut tx = app.pool.begin().await?;
    let exists = sqlx::query_scalar!(r#"select exists(select 1 from users where id = $1) as "e!""#, user_id).fetch_one(&mut *tx).await?;
    if !exists {
        return Err(ApiError::not_found("no such user"));
    }
    sqlx::query!("delete from user_totp where user_id = $1", user_id).execute(&mut *tx).await?;
    sqlx::query!("delete from recovery_codes where user_id = $1", user_id).execute(&mut *tx).await?;
    audit::record(&mut tx, &me.actor, &Entry::new("user.totp_reset", "global").resource("user", user_id)).await?;
    events::emit(&mut tx, Event { kind: kind::TOTP_RESET, project_id: None, data: json!({ "user_id": user_id }) }).await?;
    tx.commit().await?;
    // Their open sessions were authorised by a factor that no longer exists.
    session::delete_all_for_user(&app.pool, user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
