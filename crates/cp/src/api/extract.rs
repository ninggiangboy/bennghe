// SPDX-License-Identifier: AGPL-3.0-only
//! Request extractors that report problems in the API's own error format.
use axum::{extract::FromRequest, extract::rejection::JsonRejection};

use crate::error::ApiError;

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct ApiJson<T>(pub T);

impl From<JsonRejection> for ApiError {
    fn from(r: JsonRejection) -> Self {
        ApiError::bad_request(r.body_text())
    }
}
