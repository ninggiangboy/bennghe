// SPDX-License-Identifier: AGPL-3.0-only
//! Background job queue on Postgres (`graphile_worker`).
//!
//! Jobs are enqueued with `enqueue` inside the caller's transaction, so a job exists exactly when
//! the change that caused it was committed.
use graphile_worker::{IntoTaskHandlerResult, TaskHandler, Worker, WorkerContext, WorkerOptions};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool};

use crate::events::EventRecord;

pub const SCHEMA: &str = "graphile_worker";

/// Consumes domain events. Phase 8 replaces the body with notification rules; until then events
/// are only logged so the queue does not grow without bound.
#[derive(Debug, Serialize, Deserialize)]
pub struct EventTask(pub EventRecord);

impl TaskHandler for EventTask {
    const IDENTIFIER: &'static str = crate::events::TASK;

    async fn run(self, _ctx: WorkerContext) -> impl IntoTaskHandlerResult {
        tracing::info!(kind = %self.0.kind, project_id = ?self.0.project_id, "event");
        Ok::<(), String>(())
    }
}

/// Installs the queue schema (idempotent) and builds a worker on the given pool.
pub async fn init(pool: &PgPool) -> anyhow::Result<Worker> {
    Ok(WorkerOptions::default().pg_pool(pool.clone()).schema(SCHEMA).concurrency(2).define_job::<EventTask>().init().await?)
}

/// Adds a job in the caller's transaction. `job_key` replaces a pending job with the same key.
pub async fn enqueue(tx: &mut PgConnection, task: &str, payload: &serde_json::Value, job_key: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("select graphile_worker.add_job(identifier => $1, payload => $2::json, job_key => $3)")
        .bind(task)
        .bind(sqlx::types::Json(payload))
        .bind(job_key)
        .execute(tx)
        .await?;
    Ok(())
}
