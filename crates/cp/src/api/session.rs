// SPDX-License-Identifier: AGPL-3.0-only
//! Sign-in, second factor, sign-out and re-authentication.
use axum::{Extension, Json, extract::State, http::HeaderMap};
use axum_extra::extract::CookieJar;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use super::extract::ApiJson;
use crate::{
    app::AppState,
    audit::{self, Actor, Entry},
    auth::{
        password, second_factor,
        session::{self, ACTIVE_TTL, PENDING_TTL, Stage},
    },
    error::{ApiError, ApiResult, ErrorBody},
    httpserver::{Authn, ClientIp, authn::Via, cookies},
};

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct LoginResponse {
    /// `ok` once signed in, or `totp_required` when a code is still needed.
    pub status: &'static str,
    /// Send as `X-CSRF-Token` on every non-GET request. Present only when `status` is `ok`.
    pub csrf_token: Option<String>,
}

fn ok_response(csrf: String) -> LoginResponse {
    LoginResponse { status: "ok", csrf_token: Some(csrf) }
}

fn ip_of(ext: Option<Extension<ClientIp>>) -> Option<String> {
    ext.and_then(|Extension(c)| c.0).map(|i| i.to_string())
}

fn limiter_keys(email: Option<&str>, user: Option<Uuid>, ip: &Option<String>) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(e) = email {
        keys.push(format!("acct:{}", e.to_lowercase()));
    }
    if let Some(u) = user {
        keys.push(format!("user:{u}"));
    }
    if let Some(ip) = ip {
        keys.push(format!("ip:{ip}"));
    }
    keys
}

#[utoipa::path(
    operation_id = "session_login",
    post,
    path = "/api/v1/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Signed in, or a second factor is required", body = LoginResponse),
        (status = 401, description = "Wrong email or password", body = ErrorBody),
        (status = 429, description = "Too many attempts", body = ErrorBody),
    ),
    extensions(("x-permission" = json!({"action": "auth.login", "scope": "public"}))),
)]
pub async fn login(
    State(app): State<AppState>,
    ip: Option<Extension<ClientIp>>,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(req): ApiJson<LoginRequest>,
) -> ApiResult<(CookieJar, Json<LoginResponse>)> {
    let ip = ip_of(ip);
    let email = req.email.trim().to_lowercase();
    let keys = limiter_keys(Some(&email), None, &ip);
    let actor = Actor { user_id: None, token_id: None, ip: ip.clone() };
    let attempt = |reason: &'static str| Entry::new("auth.login", "public").meta(json!({ "email": email, "reason": reason }));

    if keys.iter().any(|k| app.limiter.is_blocked(k)) {
        audit::record_denied(&app.pool, &actor, &attempt("rate_limited"), "rate_limited").await;
        return Err(ApiError::too_many_requests());
    }

    let user =
        sqlx::query!("select id, password_hash, disabled_at from users where lower(email) = $1", email).fetch_optional(&app.pool).await?;
    // Verify against a dummy hash for unknown emails so response time does not reveal them.
    let hash = user.as_ref().map(|u| u.password_hash.clone()).unwrap_or_else(|| password::dummy_hash().to_owned());
    let password_ok = password::verify(req.password, hash).await;
    let user = user.filter(|u| password_ok && u.disabled_at.is_none());
    let Some(user) = user else {
        keys.iter().for_each(|k| app.limiter.record_failure(k));
        audit::record_denied(&app.pool, &actor, &attempt("bad_credentials"), "bad_credentials").await;
        return Err(ApiError::new(axum::http::StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS", "wrong email or password"));
    };

    let user_agent = headers.get("user-agent").and_then(|v| v.to_str().ok());
    let actor = Actor { user_id: Some(user.id), ..actor };
    if crate::httpserver::authn::totp_enabled(&app.pool, user.id).await? {
        let created = session::create(&app.pool, user.id, Stage::PendingSecondFactor, user_agent).await?;
        let jar = jar.add(cookies::build(&app.config, created.token, PENDING_TTL));
        return Ok((jar, Json(LoginResponse { status: "totp_required", csrf_token: None })));
    }
    let created = session::create(&app.pool, user.id, Stage::Active, user_agent).await?;
    keys.iter().for_each(|k| app.limiter.clear(k));
    let mut conn = app.pool.acquire().await?;
    audit::record(&mut conn, &actor, &Entry::new("auth.login", "public").resource("user", user.id)).await?;
    let jar = jar.add(cookies::build(&app.config, created.token, ACTIVE_TTL));
    Ok((jar, Json(ok_response(created.csrf_token))))
}

#[derive(Deserialize, ToSchema)]
pub struct CodeRequest {
    /// A 6-digit authenticator code or an unused recovery code.
    pub code: String,
}

#[utoipa::path(
    operation_id = "session_login_totp",
    post,
    path = "/api/v1/auth/login/totp",
    tag = "auth",
    request_body = CodeRequest,
    responses(
        (status = 200, description = "Signed in", body = LoginResponse),
        (status = 401, description = "No pending sign-in, or the code is wrong", body = ErrorBody),
        (status = 429, description = "Too many attempts", body = ErrorBody),
    ),
    extensions(("x-permission" = json!({"action": "auth.login_totp", "scope": "public"}))),
)]
pub async fn login_totp(
    State(app): State<AppState>,
    ip: Option<Extension<ClientIp>>,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(req): ApiJson<CodeRequest>,
) -> ApiResult<(CookieJar, Json<LoginResponse>)> {
    let ip = ip_of(ip);
    let Some(token) = cookies::read(&app.config, &headers) else { return Err(ApiError::unauthorized()) };
    let Some(pending) = session::lookup(&app.pool, &token).await?.filter(|s| s.stage == Stage::PendingSecondFactor) else {
        return Err(ApiError::unauthorized());
    };
    let actor = Actor { user_id: Some(pending.user_id), token_id: None, ip: ip.clone() };
    let keys = limiter_keys(None, Some(pending.user_id), &ip);
    let entry = |reason: &'static str| Entry::new("auth.login_totp", "public").meta(json!({ "reason": reason }));

    if keys.iter().any(|k| app.limiter.is_blocked(k)) {
        audit::record_denied(&app.pool, &actor, &entry("rate_limited"), "rate_limited").await;
        return Err(ApiError::too_many_requests());
    }
    if !second_factor::check(&app.pool, &app.secrets, pending.user_id, &req.code).await.map_err(ApiError::internal)? {
        keys.iter().for_each(|k| app.limiter.record_failure(k));
        audit::record_denied(&app.pool, &actor, &entry("bad_code"), "bad_code").await;
        return Err(ApiError::new(axum::http::StatusCode::UNAUTHORIZED, "INVALID_CODE", "wrong or already used code"));
    }
    keys.iter().for_each(|k| app.limiter.clear(k));
    let created = session::activate(&app.pool, pending.id).await?;
    let mut conn = app.pool.acquire().await?;
    audit::record(&mut conn, &actor, &Entry::new("auth.login_totp", "public").resource("user", pending.user_id)).await?;
    Ok((jar.add(cookies::build(&app.config, created.token, ACTIVE_TTL)), Json(ok_response(created.csrf_token))))
}

#[utoipa::path(
    operation_id = "session_logout",
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    responses((status = 204, description = "Signed out")),
    extensions(("x-permission" = json!({"action": "auth.logout", "scope": "self"}))),
)]
pub async fn logout(
    State(app): State<AppState>,
    Extension(me): Extension<Authn>,
    jar: CookieJar,
) -> ApiResult<(CookieJar, axum::http::StatusCode)> {
    if let Via::Session { id, .. } = me.via {
        session::delete(&app.pool, id).await?;
    }
    Ok((jar.add(cookies::clear(&app.config)), axum::http::StatusCode::NO_CONTENT))
}

#[derive(Serialize, ToSchema)]
pub struct UserSummary {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
}

#[derive(Serialize, ToSchema)]
pub struct SessionInfo {
    pub user: UserSummary,
    pub global_role: Option<String>,
    pub csrf_token: String,
    pub totp_enabled: bool,
    /// True when the installation requires a second factor and this user has not enabled one;
    /// the interface must send them to enrolment before anything else.
    pub must_enrol_2fa: bool,
    /// Until when sensitive operations are allowed without asking for the password again.
    pub reauth_valid_until: DateTime<Utc>,
}

#[utoipa::path(
    operation_id = "session_current_session",
    get,
    path = "/api/v1/auth/session",
    tag = "auth",
    responses((status = 200, body = SessionInfo), (status = 401, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "auth.session.read", "scope": "self"}))),
)]
pub async fn current_session(Extension(me): Extension<Authn>) -> ApiResult<Json<SessionInfo>> {
    let Via::Session { csrf_token, last_verified_at, .. } = me.via else {
        return Err(ApiError::forbidden("FORBIDDEN", "sessions are only available to browser logins"));
    };
    Ok(Json(SessionInfo {
        user: UserSummary { id: me.user_id, email: me.email, display_name: me.display_name },
        global_role: me.grants.global.map(|r| r.as_str().to_owned()),
        csrf_token,
        totp_enabled: me.totp_enabled,
        must_enrol_2fa: me.must_enrol_2fa,
        reauth_valid_until: last_verified_at + session::REAUTH_WINDOW,
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct ReauthRequest {
    pub password: Option<String>,
    /// Authenticator or recovery code; accepted instead of the password when 2FA is enabled.
    pub code: Option<String>,
}

#[utoipa::path(
    operation_id = "session_reauth",
    post,
    path = "/api/v1/auth/reauth",
    tag = "auth",
    request_body = ReauthRequest,
    responses((status = 204, description = "Confirmed"), (status = 401, body = ErrorBody), (status = 429, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "auth.reauth", "scope": "self"}))),
)]
pub async fn reauth(
    State(app): State<AppState>,
    Extension(me): Extension<Authn>,
    ApiJson(req): ApiJson<ReauthRequest>,
) -> ApiResult<axum::http::StatusCode> {
    let Via::Session { id, .. } = me.via else { return Err(ApiError::forbidden("FORBIDDEN", "browser session required")) };
    let keys = limiter_keys(None, Some(me.user_id), &me.actor.ip);
    let entry = |reason: &'static str| Entry::new("auth.reauth", "self").meta(json!({ "reason": reason }));
    if keys.iter().any(|k| app.limiter.is_blocked(k)) {
        audit::record_denied(&app.pool, &me.actor, &entry("rate_limited"), "rate_limited").await;
        return Err(ApiError::too_many_requests());
    }
    let ok = match (req.password, req.code) {
        (Some(pw), _) => {
            let hash = sqlx::query_scalar!("select password_hash from users where id = $1", me.user_id).fetch_one(&app.pool).await?;
            password::verify(pw, hash).await
        }
        (None, Some(code)) if me.totp_enabled => {
            second_factor::check(&app.pool, &app.secrets, me.user_id, &code).await.map_err(ApiError::internal)?
        }
        _ => return Err(ApiError::bad_request("provide `password`, or `code` when two-factor authentication is enabled")),
    };
    if !ok {
        keys.iter().for_each(|k| app.limiter.record_failure(k));
        audit::record_denied(&app.pool, &me.actor, &entry("bad_credentials"), "bad_credentials").await;
        return Err(ApiError::new(axum::http::StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS", "wrong password or code"));
    }
    keys.iter().for_each(|k| app.limiter.clear(k));
    session::mark_verified(&app.pool, id).await?;
    let mut conn = app.pool.acquire().await?;
    audit::record(&mut conn, &me.actor, &Entry::new("auth.reauth", "self").resource("user", me.user_id)).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
