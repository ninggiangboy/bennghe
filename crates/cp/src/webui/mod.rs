// SPDX-License-Identifier: AGPL-3.0-only
//! Serves the embedded single-page web interface.
use axum::{
    body::Body,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

use crate::error::ApiError;

#[derive(RustEmbed)]
#[folder = "../../web/dist/"]
struct Assets;

/// Router fallback: static files, then `index.html` for client-side routes. Unknown `/api/`
/// paths get a JSON 404 so API clients never receive HTML.
pub async fn handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path == "api" || path.starts_with("api/") {
        return ApiError::not_found("no such API route").into_response();
    }
    let wanted = if path.is_empty() { "index.html" } else { path };
    if let Some(file) = Assets::get(wanted) {
        return file_response(wanted, file);
    }
    let Some(index) = Assets::get("index.html") else {
        return (StatusCode::SERVICE_UNAVAILABLE, "The web interface is not built into this binary. Run `make web` and rebuild.")
            .into_response();
    };
    // A path with a file extension that does not exist is a real 404; anything else is a route
    // of the single-page app.
    let looks_like_file = wanted.rsplit('/').next().is_some_and(|name| name.contains('.'));
    if looks_like_file {
        return StatusCode::NOT_FOUND.into_response();
    }
    file_response("index.html", index)
}

fn file_response(name: &str, file: rust_embed::EmbeddedFile) -> Response {
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    // Vite fingerprints everything under /assets/, so those files can be cached forever.
    let cache = if name.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(file.data.into_owned()))
        .expect("static response is valid")
}

/// Security headers for every response: no framing, no sniffing, and scripts only from ourselves.
pub fn security_headers() -> [(header::HeaderName, header::HeaderValue); 4] {
    use header::HeaderValue as V;
    [
        (header::X_CONTENT_TYPE_OPTIONS, V::from_static("nosniff")),
        (header::X_FRAME_OPTIONS, V::from_static("DENY")),
        (header::REFERRER_POLICY, V::from_static("same-origin")),
        (
            header::CONTENT_SECURITY_POLICY,
            V::from_static(
                "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
            ),
        ),
    ]
}
