// SPDX-License-Identifier: AGPL-3.0-only
//! Postgres access: pool construction and schema migrations.
use std::time::Duration;

use sqlx::{PgPool, postgres::PgPoolOptions};

pub async fn connect(url: &str) -> anyhow::Result<PgPool> {
    Ok(PgPoolOptions::new().max_connections(20).acquire_timeout(Duration::from_secs(10)).connect(url).await?)
}

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    MIGRATOR.run(pool).await?;
    Ok(())
}
