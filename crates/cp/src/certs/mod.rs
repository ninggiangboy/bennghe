// SPDX-License-Identifier: AGPL-3.0-only
//! TLS for the control plane's own domain: certificates over ACME HTTP-01, held in a resolver
//! that is replaced while the server keeps running. Phase 4 extends the same module to the
//! domains of deployed apps.
mod acme;
mod resolver;

use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use axum::{
    Router,
    extract::{ConnectInfo, Path, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Redirect},
    routing::get,
};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto,
    service::TowerToHyperService,
};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

pub use acme::{AcmeIssuer, ChallengeMap, manage};
pub use resolver::{CertResolver, certified_key};

use crate::{config::Config, server::Runtime};

#[derive(Clone)]
struct HttpState {
    challenges: ChallengeMap,
    domain: String,
}

async fn challenge(State(s): State<HttpState>, Path(token): Path<String>) -> impl IntoResponse {
    match s.challenges.get(&token) {
        Some(key_authorization) => (StatusCode::OK, key_authorization).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Everything else on port 80 goes to HTTPS on the configured domain. The target is built from
/// configuration, never from the `Host` header, so it cannot be turned into an open redirect.
async fn to_https(State(s): State<HttpState>, uri: Uri) -> Redirect {
    let path = uri.path_and_query().map_or("/", |p| p.as_str());
    Redirect::permanent(&format!("https://{}{path}", s.domain))
}

pub async fn serve_with_acme(
    config: Arc<Config>,
    _rt: Runtime,
    router: Router,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let domain = config.public_domain.clone().context("BN_PUBLIC_DOMAIN is required for ACME")?.to_ascii_lowercase();
    let challenges = ChallengeMap::default();
    let resolver = Arc::new(CertResolver::default());

    let issuer = AcmeIssuer::new(&config, challenges.clone())?;
    tokio::spawn(manage(issuer, resolver.clone(), domain.clone()));

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown.await;
        let _ = stop_tx.send(true);
    });

    let http_app = Router::new()
        .route("/.well-known/acme-challenge/{token}", get(challenge))
        // Lets the container health check and the installer see the process is up before a
        // certificate exists, when HTTPS cannot answer yet.
        .route("/api/v1/health", get(|| async { axum::Json(serde_json::json!({ "status": "ok" })) }))
        .fallback(to_https)
        .with_state(HttpState { challenges, domain });
    let http_listener = TcpListener::bind(config.http_addr).await.with_context(|| format!("binding {}", config.http_addr))?;
    tracing::info!(addr = %http_listener.local_addr()?, "listening for ACME challenges and redirects");
    let mut http_stop = stop_rx.clone();
    let http = tokio::spawn(async move {
        axum::serve(http_listener, http_app)
            .with_graceful_shutdown(async move {
                let _ = http_stop.wait_for(|s| *s).await;
            })
            .await
    });

    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let listener = TcpListener::bind(config.https_addr).await.with_context(|| format!("binding {}", config.https_addr))?;
    tracing::info!(addr = %listener.local_addr()?, "listening for HTTPS");
    serve_tls(listener, acceptor, router, stop_rx).await;
    http.await??;
    Ok(())
}

async fn serve_tls(listener: TcpListener, acceptor: TlsAcceptor, router: Router, mut stop: tokio::sync::watch::Receiver<bool>) {
    loop {
        let (tcp, peer): (_, SocketAddr) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(a) => a,
                Err(e) => { tracing::warn!(error = %e, "accept failed"); continue; }
            },
            _ = stop.wait_for(|s| *s) => return,
        };
        let acceptor = acceptor.clone();
        let app = router.clone().layer(axum::Extension(ConnectInfo(peer)));
        tokio::spawn(async move {
            // Handshakes fail until the first certificate is issued; that is expected noise.
            let Ok(stream) = acceptor.accept(tcp).await else { return };
            let service = TowerToHyperService::new(app);
            let _ = auto::Builder::new(TokioExecutor::new()).serve_connection_with_upgrades(TokioIo::new(stream), service).await;
        });
    }
}
