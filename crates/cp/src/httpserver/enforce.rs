// SPDX-License-Identifier: AGPL-3.0-only
//! The gate every request passes: authentication, CSRF, second-factor enrolment, authorisation
//! and recent re-authentication, in that order. Every refusal is written to the audit log.
use axum::{
    extract::{MatchedPath, RawPathParams, Request, State},
    http::Method,
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use super::{
    authn::{self, Authn, Via},
    context::ClientIp,
};
use crate::{
    app::AppState,
    audit::{self, Actor, Entry},
    auth::session::{REAUTH_WINDOW, Stage},
    error::ApiError,
    rbac::{Scope, Subject, decide, policy},
    util::ct_eq,
};

pub async fn enforce(State(app): State<AppState>, matched: MatchedPath, params: RawPathParams, mut req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let template = matched.as_str().to_owned();
    let ip = req.extensions().get::<ClientIp>().and_then(|c| c.0).map(|i| i.to_string());

    let Some(perm) = app.perms.lookup(method.as_str(), &template).cloned() else {
        // Unreachable while `PermTable::from_openapi` succeeds at startup; fail closed regardless.
        tracing::error!(%template, "route has no declared permission");
        return ApiError::forbidden("FORBIDDEN", "route has no declared permission").into_response();
    };
    if perm.scope == Scope::Public {
        return next.run(req).await;
    }

    let scope_id: Option<Uuid> =
        perm.scope.path_param().and_then(|name| params.iter().find(|(k, _)| *k == name)).and_then(|(_, v)| v.parse().ok());
    let meta = |reason: &str| json!({ "reason": reason, "method": method.as_str(), "route": template });
    let deny = |actor: Actor, error: ApiError, reason: &'static str| {
        let app = app.clone();
        let entry_scope = perm.scope;
        let action = perm.action.clone();
        let metadata = meta(reason);
        async move {
            let entry = Entry::new(&action, entry_scope.as_str()).scope_id(scope_id).meta(metadata);
            audit::record_denied(&app.pool, &actor, &entry, reason).await;
            error.into_response()
        }
    };

    let authn = match authn::authenticate(&app, req.headers(), ip.clone()).await {
        Ok(Some(a)) => a,
        Ok(None) => return deny(Actor { ip, ..Actor::default() }, ApiError::unauthorized(), "unauthenticated").await,
        Err(e) => return e.into_response(),
    };
    let actor = authn.actor.clone();

    if let Via::Session { stage: Stage::PendingSecondFactor, .. } = &authn.via {
        return deny(actor, ApiError::unauthorized(), "second_factor_pending").await;
    }

    // Browser sessions carry cookies, so unsafe methods must prove they came from our own page.
    if let Via::Session { csrf_token, .. } = &authn.via
        && !matches!(method, Method::GET | Method::HEAD | Method::OPTIONS)
    {
        let sent = req.headers().get("x-csrf-token").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !ct_eq(sent.as_bytes(), csrf_token.as_bytes()) {
            return deny(actor, ApiError::forbidden("CSRF_INVALID", "missing or invalid CSRF token"), "csrf").await;
        }
    }

    if authn.must_enrol_2fa {
        match &authn.via {
            Via::Token { .. } => {
                return deny(
                    actor,
                    ApiError::forbidden("TWO_FACTOR_REQUIRED", "the token owner must enable two-factor authentication"),
                    "two_factor_required",
                )
                .await;
            }
            Via::Session { .. } if !policy::ENROLMENT_ACTIONS.contains(&perm.action.as_str()) => {
                return deny(
                    actor,
                    ApiError::forbidden("TWO_FACTOR_ENROLMENT_REQUIRED", "enable two-factor authentication to continue"),
                    "two_factor_enrolment",
                )
                .await;
            }
            Via::Session { .. } => {}
        }
    }

    let Some(rule) = policy::rule_for(&perm.action) else {
        return deny(actor, ApiError::forbidden("FORBIDDEN", "action is not in the permission matrix"), "no_rule").await;
    };
    let subject = match &authn.via {
        Via::Session { .. } => Subject::User(&authn.grants),
        Via::Token { role, project_ids, .. } => Subject::Token { role: *role, project_ids, owner: &authn.grants },
    };
    if let Err(denial) = decide(rule, &subject, scope_id) {
        return deny(actor, ApiError::forbidden("FORBIDDEN", "you do not have permission to do this"), denial.reason()).await;
    }

    if perm.reauth {
        match &authn.via {
            Via::Token { .. } => {
                return deny(
                    actor,
                    ApiError::forbidden("FORBIDDEN", "this operation is only available in the web interface"),
                    "reauth_not_possible_with_token",
                )
                .await;
            }
            Via::Session { last_verified_at, .. } if Utc::now() - *last_verified_at > REAUTH_WINDOW => {
                return deny(actor, ApiError::forbidden("REAUTH_REQUIRED", "confirm your password or code to continue"), "reauth_required")
                    .await;
            }
            Via::Session { .. } => {}
        }
    }

    req.extensions_mut().insert(authn);
    next.run(req).await
}

pub type Caller = axum::Extension<Authn>;
