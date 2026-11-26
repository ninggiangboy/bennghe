// SPDX-License-Identifier: AGPL-3.0-only
//! Shared application state handed to every handler and to the enforcement middleware.
use std::sync::Arc;

use axum::Router;
use sqlx::PgPool;
use utoipa::openapi::OpenApi;

use crate::{
    auth::ratelimit::RateLimiter,
    config::Config,
    rbac::PermTable,
    secrets::{Identity, Secrets},
};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub secrets: Arc<Secrets>,
    pub config: Arc<Config>,
    pub identity: Arc<Identity>,
    pub perms: Arc<PermTable>,
    pub limiter: Arc<RateLimiter>,
}

/// Builds the API router and the OpenAPI document that describes it. Both come from the same
/// route declarations, so the document cannot drift from what is served.
pub fn build(pool: PgPool, secrets: Arc<Secrets>, config: Arc<Config>, identity: Arc<Identity>) -> anyhow::Result<(Router, OpenApi)> {
    let (router, openapi) = crate::api::router().split_for_parts();
    let perms =
        PermTable::from_openapi(&openapi).map_err(|problems| anyhow::anyhow!("invalid route permissions:\n{}", problems.join("\n")))?;
    let state = AppState { pool, secrets, config, identity, perms: Arc::new(perms), limiter: Arc::new(RateLimiter::login()) };
    let router = router
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), crate::httpserver::enforce::enforce))
        .layer(axum::middleware::from_fn_with_state(state.clone(), crate::httpserver::context::attach))
        .with_state(state);
    Ok((router, openapi))
}
