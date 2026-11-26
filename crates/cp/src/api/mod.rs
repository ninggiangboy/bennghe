// SPDX-License-Identifier: AGPL-3.0-only
//! The HTTP API. Routes are registered together with their OpenAPI operations, so the generated
//! `api/openapi.yaml` and the served routes always agree.
mod account;
mod admin;
mod extract;
mod health;
mod projects;
mod session;
mod setup;

use utoipa::{
    Modify, OpenApi,
    openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme},
};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{app::AppState, error::ErrorBody};

struct SecuritySchemes;

impl Modify for SecuritySchemes {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme("bearer_token", SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)));
        components.add_security_scheme("session_cookie", SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("__Host-bn_sid"))));
        components.add_security_scheme("csrf_token", SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-CSRF-Token"))));
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Bennghe API",
        description = "Control plane API. Every operation declares `x-permission` with the action, the scope it applies to and whether it needs a recent re-authentication.",
        license(name = "AGPL-3.0-only"),
    ),
    components(schemas(ErrorBody)),
    modifiers(&SecuritySchemes),
)]
struct ApiDoc;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(health::health))
        .routes(routes!(setup::status))
        .routes(routes!(setup::create))
        .routes(routes!(session::login))
        .routes(routes!(session::login_totp))
        .routes(routes!(session::logout))
        .routes(routes!(session::current_session))
        .routes(routes!(session::reauth))
        .routes(routes!(account::totp_begin, account::totp_disable))
        .routes(routes!(account::totp_confirm))
        .routes(routes!(account::token_list, account::token_create))
        .routes(routes!(account::token_revoke))
        .routes(routes!(admin::security_get, admin::security_put))
        .routes(routes!(admin::audit_list))
        .routes(routes!(admin::totp_reset))
        .routes(routes!(projects::list, projects::create))
        .routes(routes!(projects::get, projects::update, projects::delete))
        .routes(routes!(projects::members))
        .routes(routes!(projects::set_member, projects::remove_member))
}

/// The OpenAPI document, without building any state. Used to generate `api/openapi.yaml`.
pub fn openapi() -> utoipa::openapi::OpenApi {
    router().split_for_parts().1
}
