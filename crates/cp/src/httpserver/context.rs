// SPDX-License-Identifier: AGPL-3.0-only
//! Per-request context that every later layer and handler can rely on.
use std::net::{IpAddr, SocketAddr};

use axum::{
    extract::{ConnectInfo, Request, State},
    middleware::Next,
    response::Response,
};

use super::ip::client_ip;
use crate::app::AppState;

#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub Option<IpAddr>);

pub async fn attach(State(app): State<AppState>, mut req: Request, next: Next) -> Response {
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let ip = client_ip(req.headers(), peer, &app.config.trusted_proxies);
    req.extensions_mut().insert(ClientIp(ip));
    next.run(req).await
}
