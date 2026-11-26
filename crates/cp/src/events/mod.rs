// SPDX-License-Identifier: AGPL-3.0-only
//! Domain events. Every phase emits through `emit`, inside the transaction of the change itself;
//! phase 8 adds the consumer that turns them into notifications. There is no separate bus.
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

/// Job identifier under which events travel through the queue.
pub const TASK: &str = "bn_event";

/// Event names. New phases add constants here so the vocabulary stays in one place.
pub mod kind {
    pub const PROJECT_CREATED: &str = "project.created";
    pub const PROJECT_DELETED: &str = "project.deleted";
    pub const MEMBER_SET: &str = "project.member_set";
    pub const MEMBER_REMOVED: &str = "project.member_removed";
    pub const TOTP_RESET: &str = "user.totp_reset";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventRecord {
    pub kind: String,
    pub project_id: Option<Uuid>,
    pub data: serde_json::Value,
}

pub struct Event {
    pub kind: &'static str,
    pub project_id: Option<Uuid>,
    pub data: serde_json::Value,
}

pub async fn emit(tx: &mut PgConnection, event: Event) -> Result<(), sqlx::Error> {
    let record = EventRecord { kind: event.kind.to_owned(), project_id: event.project_id, data: event.data };
    let payload = serde_json::to_value(EventTask::wrap(record)).expect("event record serialises");
    crate::jobs::enqueue(tx, TASK, &payload, None).await
}

use crate::jobs::EventTask;

impl EventTask {
    fn wrap(record: EventRecord) -> Self {
        Self(record)
    }
}
