// SPDX-License-Identifier: AGPL-3.0-only
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
}

#[utoipa::path(
    operation_id = "health_health",
    get,
    path = "/api/v1/health",
    tag = "system",
    responses((status = 200, description = "The control plane is up", body = Health)),
    extensions(("x-permission" = json!({"action": "health.read", "scope": "public"}))),
)]
pub async fn health() -> Json<Health> {
    Json(Health { status: "ok", version: env!("CARGO_PKG_VERSION") })
}
