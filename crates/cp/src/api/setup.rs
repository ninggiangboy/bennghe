// SPDX-License-Identifier: AGPL-3.0-only
//! First-run setup of the owner account.
use axum::{Extension, Json, extract::State};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use super::{extract::ApiJson, session::LoginResponse};
use crate::{
    app::AppState,
    audit::{self, Actor, Entry},
    auth::{
        password,
        session::{self, ACTIVE_TTL, Stage},
        setup,
    },
    error::{ApiError, ApiResult, ErrorBody},
    httpserver::{ClientIp, cookies},
};

#[derive(Serialize, ToSchema)]
pub struct SetupStatus {
    pub needs_setup: bool,
}

#[utoipa::path(
    operation_id = "setup_status",
    get,
    path = "/api/v1/setup/status",
    tag = "setup",
    responses((status = 200, body = SetupStatus)),
    extensions(("x-permission" = json!({"action": "setup.status", "scope": "public"}))),
)]
pub async fn status(State(app): State<AppState>) -> ApiResult<Json<SetupStatus>> {
    Ok(Json(SetupStatus { needs_setup: setup::needs_setup(&app.pool).await? }))
}

#[derive(Deserialize, ToSchema)]
pub struct SetupRequest {
    /// The one-time token printed on the host by the install script.
    pub token: String,
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
}

pub fn validate_email(email: &str) -> Result<(), ApiError> {
    let ok = email.len() <= 254
        && email.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.'))
        && !email.contains(char::is_whitespace);
    if ok { Ok(()) } else { Err(ApiError::bad_request("email address is not valid")) }
}

#[utoipa::path(
    operation_id = "setup_create",
    post,
    path = "/api/v1/setup",
    tag = "setup",
    request_body = SetupRequest,
    responses(
        (status = 200, description = "Owner created and signed in", body = LoginResponse),
        (status = 403, description = "Missing or wrong setup token, or setup already done", body = ErrorBody),
    ),
    extensions(("x-permission" = json!({"action": "setup.create", "scope": "public"}))),
)]
pub async fn create(
    State(app): State<AppState>,
    ip: Option<Extension<ClientIp>>,
    jar: CookieJar,
    ApiJson(req): ApiJson<SetupRequest>,
) -> ApiResult<(CookieJar, Json<LoginResponse>)> {
    let ip = ip.and_then(|Extension(c)| c.0).map(|i| i.to_string());
    let actor = Actor { user_id: None, token_id: None, ip: ip.clone() };
    let key = format!("setup:{}", ip.clone().unwrap_or_default());
    let entry = |reason: &str| Entry::new("setup.create", "public").meta(json!({ "reason": reason }));
    if app.limiter.is_blocked(&key) {
        return Err(ApiError::too_many_requests());
    }
    let email = req.email.trim().to_lowercase();
    validate_email(&email)?;
    password::validate_new(&req.password).map_err(ApiError::bad_request)?;
    let password_hash = password::hash(req.password).await.map_err(ApiError::internal)?;

    let mut tx = app.pool.begin().await?;
    // One setup at a time, so two valid tokens cannot create two owners.
    sqlx::query!("select pg_advisory_xact_lock(7001)").execute(&mut *tx).await?;
    let has_users = sqlx::query_scalar!(r#"select exists(select 1 from users) as "e!""#).fetch_one(&mut *tx).await?;
    if has_users {
        drop(tx);
        audit::record_denied(&app.pool, &actor, &entry("already_set_up"), "already_set_up").await;
        return Err(ApiError::forbidden("SETUP_COMPLETE", "this installation is already set up"));
    }
    if !setup::consume(&mut tx, &req.token).await? {
        drop(tx);
        app.limiter.record_failure(&key);
        audit::record_denied(&app.pool, &actor, &entry("bad_setup_token"), "bad_setup_token").await;
        return Err(ApiError::forbidden("INVALID_SETUP_TOKEN", "the setup token is wrong or already used"));
    }
    let display_name = req.display_name.unwrap_or_default();
    let user_id = sqlx::query_scalar!(
        "insert into users (email, display_name, password_hash) values ($1, $2, $3) returning id",
        email,
        display_name,
        password_hash
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query!("insert into memberships (user_id, scope, role) values ($1, 'global', 'owner')", user_id).execute(&mut *tx).await?;
    let actor = Actor { user_id: Some(user_id), ..actor };
    audit::record(&mut tx, &actor, &Entry::new("setup.create", "public").resource("user", user_id)).await?;
    tx.commit().await?;

    let created = session::create(&app.pool, user_id, Stage::Active, None).await?;
    Ok((
        jar.add(cookies::build(&app.config, created.token, ACTIVE_TTL)),
        Json(LoginResponse { status: "ok", csrf_token: Some(created.csrf_token) }),
    ))
}
