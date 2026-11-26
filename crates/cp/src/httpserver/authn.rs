// SPDX-License-Identifier: AGPL-3.0-only
//! Works out who is calling: a bearer API token or a browser session.
use axum::http::{HeaderMap, header::AUTHORIZATION};
use uuid::Uuid;

use super::cookies;
use crate::{
    app::AppState,
    audit::Actor,
    auth::{api_token, session},
    error::ApiError,
    rbac::{Grants, Role, Scope},
};

#[derive(Debug, Clone)]
pub enum Via {
    Session { id: Uuid, csrf_token: String, last_verified_at: chrono::DateTime<chrono::Utc>, stage: session::Stage },
    Token { id: Uuid, role: Role, project_ids: Vec<Uuid> },
}

/// Everything handlers need to know about the caller. Placed in request extensions by the
/// enforcement middleware once the request is authenticated and authorised.
#[derive(Debug, Clone)]
pub struct Authn {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub grants: Grants,
    pub via: Via,
    pub totp_enabled: bool,
    /// The installation requires a second factor from this user and they have none yet.
    pub must_enrol_2fa: bool,
    pub actor: Actor,
}

impl Authn {
    pub fn token_id(&self) -> Option<Uuid> {
        match &self.via {
            Via::Token { id, .. } => Some(*id),
            Via::Session { .. } => None,
        }
    }
}

pub async fn require_2fa_for_admins(pool: &sqlx::PgPool) -> Result<bool, sqlx::Error> {
    let value = sqlx::query_scalar!("select value from settings where key = 'require_2fa_for_admins'").fetch_optional(pool).await?;
    Ok(value.and_then(|v| v.as_bool()).unwrap_or(false))
}

pub async fn load_grants(pool: &sqlx::PgPool, user_id: Uuid) -> Result<Grants, sqlx::Error> {
    let rows = sqlx::query!("select scope, scope_id, role from memberships where user_id = $1", user_id).fetch_all(pool).await?;
    let mut grants = Grants::default();
    for row in rows {
        let Some(role) = Role::parse(&row.role) else { continue };
        match (row.scope.as_str(), row.scope_id) {
            ("global", None) => grants.global = Some(role),
            ("project", Some(id)) => {
                grants.scoped.insert((Scope::Project, id), role);
            }
            ("cluster", Some(id)) => {
                grants.scoped.insert((Scope::Cluster, id), role);
            }
            _ => {}
        }
    }
    Ok(grants)
}

pub async fn totp_enabled(pool: &sqlx::PgPool, user_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(r#"select exists(select 1 from user_totp where user_id = $1 and confirmed_at is not null) as "e!""#, user_id)
        .fetch_one(pool)
        .await
}

/// Returns `None` when the request carries no valid credentials.
pub async fn authenticate(app: &AppState, headers: &HeaderMap, ip: Option<String>) -> Result<Option<Authn>, ApiError> {
    let bearer = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .map(str::trim);

    let (user_id, via) = if let Some(presented) = bearer {
        // A bearer header is the only credential considered, so a stray cookie cannot widen it.
        match api_token::authenticate(&app.pool, presented).await? {
            Some(t) => {
                let role = Role::parse(&t.role).ok_or_else(|| ApiError::internal("token has an unknown role"))?;
                (t.user_id, Via::Token { id: t.id, role, project_ids: t.project_ids })
            }
            None => return Ok(None),
        }
    } else if let Some(cookie) = cookies::read(&app.config, headers) {
        match session::lookup(&app.pool, &cookie).await? {
            Some(s) => {
                (s.user_id, Via::Session { id: s.id, csrf_token: s.csrf_token, last_verified_at: s.last_verified_at, stage: s.stage })
            }
            None => return Ok(None),
        }
    } else {
        return Ok(None);
    };

    let user = sqlx::query!("select email, display_name from users where id = $1 and disabled_at is null", user_id)
        .fetch_optional(&app.pool)
        .await?;
    let Some(user) = user else { return Ok(None) };
    let grants = load_grants(&app.pool, user_id).await?;
    let totp = totp_enabled(&app.pool, user_id).await?;
    let must_enrol_2fa = !totp && grants.is_admin_anywhere() && require_2fa_for_admins(&app.pool).await?;
    let token_id = match &via {
        Via::Token { id, .. } => Some(*id),
        Via::Session { .. } => None,
    };
    let actor = Actor { user_id: Some(user_id), token_id, ip };
    Ok(Some(Authn { user_id, email: user.email, display_name: user.display_name, grants, via, totp_enabled: totp, must_enrol_2fa, actor }))
}
