// SPDX-License-Identifier: AGPL-3.0-only
//! Process start-up for `controlplane serve`.
use std::sync::Arc;

use anyhow::Context;
use sqlx::PgPool;
use tokio::net::TcpListener;

use crate::{
    app,
    config::{Config, TlsMode},
    jobs,
    secrets::{Identity, Secrets},
    store,
};

/// Everything `serve` and the admin commands need once the database is reachable.
pub struct Runtime {
    pub pool: PgPool,
    pub secrets: Arc<Secrets>,
    pub identity: Arc<Identity>,
}

pub async fn prepare(config: &Config) -> anyhow::Result<Runtime> {
    let pool = store::connect(&config.database_url).await.context("connecting to Postgres")?;
    store::migrate(&pool).await.context("running migrations")?;
    let secrets = Secrets::load(&config.master_key_path()).context("loading master.key (run `controlplane init` first)")?;
    secrets.verify_against(&pool).await?;
    let identity = Identity::ensure(&pool, &config.signing_key_path()).await.context("loading signing.key")?;
    Ok(Runtime { pool, secrets: Arc::new(secrets), identity: Arc::new(identity) })
}

pub async fn serve(config: Config) -> anyhow::Result<()> {
    let config = Arc::new(config);
    let rt = prepare(&config).await?;
    tracing::info!(instance_id = %rt.identity.instance_id, epoch = %rt.identity.epoch, "control plane identity ready");

    let worker = jobs::init(&rt.pool).await.context("starting the job queue")?;
    tokio::spawn(async move {
        if let Err(e) = worker.run().await {
            tracing::error!(error = %e, "job worker stopped");
        }
    });
    let purge_pool = rt.pool.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            if let Err(e) = crate::auth::session::purge_expired(&purge_pool).await {
                tracing::warn!(error = %e, "purging expired sessions failed");
            }
        }
    });

    let (api, _) = app::build(rt.pool.clone(), rt.secrets.clone(), config.clone(), rt.identity.clone())?;
    let mut router = api.fallback(crate::webui::handler);
    for (name, value) in crate::webui::security_headers() {
        router = router.layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(name, value));
    }

    match config.tls_mode {
        TlsMode::Off => {
            let listener = TcpListener::bind(config.http_addr).await.with_context(|| format!("binding {}", config.http_addr))?;
            tracing::info!(addr = %listener.local_addr()?, "listening (plain HTTP, TLS is terminated by a proxy)");
            axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .with_graceful_shutdown(shutdown_signal())
                .await?;
        }
        TlsMode::Acme => crate::certs::serve_with_acme(config, rt, router, shutdown_signal()).await?,
    }
    Ok(())
}

pub async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    tracing::info!("shutting down");
}
