// SPDX-License-Identifier: AGPL-3.0-only
//! The signed-in user's own security settings: two-factor authentication and API tokens.
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{extract::ApiJson, session::CodeRequest};
use crate::{
    app::AppState,
    audit::{self, Entry},
    auth::{api_token, recovery, second_factor, totp},
    error::{ApiError, ApiResult, ErrorBody},
    httpserver::{Authn, authn},
    rbac::{Role, Scope},
};

#[derive(Serialize, ToSchema)]
pub struct TotpEnrolment {
    /// Base32 secret for manual entry.
    pub secret: String,
    /// `otpauth://` URI to render as a QR code.
    pub otpauth_uri: String,
}

#[utoipa::path(
    operation_id = "account_totp_begin",
    post,
    path = "/api/v1/account/totp",
    tag = "account",
    responses((status = 200, body = TotpEnrolment), (status = 409, description = "Already enabled", body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "account.totp.begin", "scope": "self"}))),
)]
pub async fn totp_begin(State(app): State<AppState>, Extension(me): Extension<Authn>) -> ApiResult<Json<TotpEnrolment>> {
    let secret = totp::generate_secret();
    let sealed = app.secrets.seal(&second_factor::aad(me.user_id), &secret).map_err(ApiError::internal)?;
    let written = sqlx::query!(
        "insert into user_totp (user_id, secret_ciphertext, key_id) values ($1, $2, $3)
         on conflict (user_id) do update
            set secret_ciphertext = excluded.secret_ciphertext, key_id = excluded.key_id, last_used_step = 0, created_at = now()
          where user_totp.confirmed_at is null",
        me.user_id,
        sealed.data,
        sealed.key_id
    )
    .execute(&app.pool)
    .await?;
    if written.rows_affected() == 0 {
        return Err(ApiError::conflict("two-factor authentication is already enabled"));
    }
    Ok(Json(TotpEnrolment { secret: totp::encode_secret(&secret), otpauth_uri: totp::otpauth_uri("Bennghe", &me.email, &secret) }))
}

#[derive(Serialize, ToSchema)]
pub struct RecoveryCodes {
    /// Shown once. Each works a single time if the authenticator is lost.
    pub recovery_codes: Vec<String>,
}

#[utoipa::path(
    operation_id = "account_totp_confirm",
    post,
    path = "/api/v1/account/totp/confirm",
    tag = "account",
    request_body = CodeRequest,
    responses((status = 200, body = RecoveryCodes), (status = 400, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "account.totp.confirm", "scope": "self"}))),
)]
pub async fn totp_confirm(
    State(app): State<AppState>,
    Extension(me): Extension<Authn>,
    ApiJson(req): ApiJson<CodeRequest>,
) -> ApiResult<Json<RecoveryCodes>> {
    let key = format!("user:{}", me.user_id);
    if app.limiter.is_blocked(&key) {
        return Err(ApiError::too_many_requests());
    }
    let mut tx = app.pool.begin().await?;
    let row =
        sqlx::query!("select secret_ciphertext, key_id from user_totp where user_id = $1 and confirmed_at is null for update", me.user_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| ApiError::bad_request("start enrolment first"))?;
    let secret = app.secrets.open(&row.key_id, &second_factor::aad(me.user_id), &row.secret_ciphertext).map_err(ApiError::internal)?;
    let now = u64::try_from(Utc::now().timestamp()).unwrap_or(0);
    let Some(step) = totp::verify(&secret, &req.code, now, 0) else {
        app.limiter.record_failure(&key);
        return Err(ApiError::bad_request("that code is not valid; check the clock on your device"));
    };
    let step = i64::try_from(step).map_err(ApiError::internal)?;
    sqlx::query!("update user_totp set confirmed_at = now(), last_used_step = $2 where user_id = $1", me.user_id, step)
        .execute(&mut *tx)
        .await?;
    let codes = recovery::generate();
    recovery::replace_all(&mut tx, me.user_id, &codes).await.map_err(ApiError::internal)?;
    audit::record(&mut tx, &me.actor, &Entry::new("account.totp.confirm", "self").resource("user", me.user_id)).await?;
    tx.commit().await?;
    app.limiter.clear(&key);
    Ok(Json(RecoveryCodes { recovery_codes: codes }))
}

#[utoipa::path(
    operation_id = "account_totp_disable",
    delete,
    path = "/api/v1/account/totp",
    tag = "account",
    responses((status = 204, description = "Disabled"), (status = 403, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "account.totp.disable", "scope": "self", "reauth": true}))),
)]
pub async fn totp_disable(State(app): State<AppState>, Extension(me): Extension<Authn>) -> ApiResult<StatusCode> {
    if me.grants.is_admin_anywhere() && authn::require_2fa_for_admins(&app.pool).await? {
        return Err(ApiError::forbidden("TWO_FACTOR_REQUIRED", "this installation requires two-factor authentication for your role"));
    }
    let mut tx = app.pool.begin().await?;
    sqlx::query!("delete from user_totp where user_id = $1", me.user_id).execute(&mut *tx).await?;
    sqlx::query!("delete from recovery_codes where user_id = $1", me.user_id).execute(&mut *tx).await?;
    audit::record(&mut tx, &me.actor, &Entry::new("account.totp.disable", "self").resource("user", me.user_id)).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
pub struct ApiTokenInfo {
    pub id: Uuid,
    pub name: String,
    /// Last four characters, to recognise the token.
    pub hint: String,
    pub role: String,
    pub project_ids: Vec<Uuid>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[utoipa::path(
    operation_id = "account_token_list",
    get,
    path = "/api/v1/account/api-tokens",
    tag = "account",
    responses((status = 200, body = Vec<ApiTokenInfo>)),
    extensions(("x-permission" = json!({"action": "account.api_token.list", "scope": "self"}))),
)]
pub async fn token_list(State(app): State<AppState>, Extension(me): Extension<Authn>) -> ApiResult<Json<Vec<ApiTokenInfo>>> {
    let rows = sqlx::query!(
        r#"select id, name, token_hint, role, project_ids,
                  expires_at as "expires_at: DateTime<Utc>", last_used_at as "last_used_at: DateTime<Utc>",
                  revoked_at as "revoked_at: DateTime<Utc>", created_at as "created_at: DateTime<Utc>"
             from api_tokens where user_id = $1 order by created_at desc"#,
        me.user_id
    )
    .fetch_all(&app.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ApiTokenInfo {
                id: r.id,
                name: r.name,
                hint: r.token_hint,
                role: r.role,
                project_ids: r.project_ids,
                expires_at: r.expires_at,
                last_used_at: r.last_used_at,
                revoked_at: r.revoked_at,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateTokenRequest {
    pub name: String,
    /// Projects the token may act in. Each must be one the creator belongs to.
    pub project_ids: Vec<Uuid>,
    /// Highest role the token may use: `admin`, `developer` or `viewer`.
    pub role: String,
    /// Lifetime in days, 1 to 365.
    pub expires_in_days: i64,
}

#[derive(Serialize, ToSchema)]
pub struct CreatedToken {
    pub id: Uuid,
    /// The token itself. It is never shown again.
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

#[utoipa::path(
    operation_id = "account_token_create",
    post,
    path = "/api/v1/account/api-tokens",
    tag = "account",
    request_body = CreateTokenRequest,
    responses((status = 201, body = CreatedToken), (status = 400, body = ErrorBody), (status = 403, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "account.api_token.create", "scope": "self", "reauth": true}))),
)]
pub async fn token_create(
    State(app): State<AppState>,
    Extension(me): Extension<Authn>,
    ApiJson(req): ApiJson<CreateTokenRequest>,
) -> ApiResult<(StatusCode, Json<CreatedToken>)> {
    let name = req.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request("name must be 1 to 100 characters"));
    }
    let role = Role::parse(&req.role)
        .filter(|r| *r != Role::Owner)
        .ok_or_else(|| ApiError::bad_request("role must be admin, developer or viewer"))?;
    if !(1..=api_token::MAX_LIFETIME_DAYS).contains(&req.expires_in_days) {
        return Err(ApiError::bad_request("expires_in_days must be between 1 and 365"));
    }
    let mut project_ids = req.project_ids;
    project_ids.sort();
    project_ids.dedup();
    if project_ids.is_empty() || project_ids.len() > 50 {
        return Err(ApiError::bad_request("choose between 1 and 50 projects"));
    }
    let existing =
        sqlx::query_scalar!(r#"select count(*) as "n!" from projects where id = any($1)"#, &project_ids).fetch_one(&app.pool).await?;
    if usize::try_from(existing).ok() != Some(project_ids.len()) {
        return Err(ApiError::not_found("one of the projects does not exist"));
    }
    for id in &project_ids {
        // A token can never carry more power than its creator has in that project.
        if me.grants.role_at(Scope::Project, *id).is_none_or(|have| have < role) {
            return Err(ApiError::forbidden("FORBIDDEN", "your role in one of those projects is lower than the token role"));
        }
    }
    let generated = api_token::generate();
    let days = i32::try_from(req.expires_in_days).map_err(ApiError::internal)?;
    let mut tx = app.pool.begin().await?;
    let created = sqlx::query!(
        r#"insert into api_tokens (user_id, name, token_hash, token_hint, role, project_ids, expires_at)
           values ($1, $2, $3, $4, $5, $6, now() + make_interval(days => $7))
           returning id, expires_at as "expires_at: DateTime<Utc>""#,
        me.user_id,
        name,
        generated.hash,
        generated.hint,
        role.as_str(),
        &project_ids,
        days
    )
    .fetch_one(&mut *tx)
    .await?;
    let (id, expires_at) = (created.id, created.expires_at);
    let meta = json!({ "name": name, "role": role.as_str(), "project_ids": project_ids, "expires_at": expires_at });
    audit::record(&mut tx, &me.actor, &Entry::new("account.api_token.create", "self").resource("api_token", id).meta(meta)).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(CreatedToken { id, token: generated.plaintext, expires_at })))
}

#[utoipa::path(
    operation_id = "account_token_revoke",
    delete,
    path = "/api/v1/account/api-tokens/{token_id}",
    tag = "account",
    params(("token_id" = Uuid, Path, description = "Token to revoke")),
    responses((status = 204, description = "Revoked"), (status = 404, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "account.api_token.revoke", "scope": "self"}))),
)]
pub async fn token_revoke(
    State(app): State<AppState>,
    Extension(me): Extension<Authn>,
    Path(token_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let mut tx = app.pool.begin().await?;
    let done = sqlx::query!(
        "update api_tokens set revoked_at = now() where id = $1 and user_id = $2 and revoked_at is null",
        token_id,
        me.user_id
    )
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("no such active token"));
    }
    audit::record(&mut tx, &me.actor, &Entry::new("account.api_token.revoke", "self").resource("api_token", token_id)).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
