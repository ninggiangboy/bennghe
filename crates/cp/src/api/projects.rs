// SPDX-License-Identifier: AGPL-3.0-only
//! Projects: the isolation boundary. A role in one project grants nothing in another.
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use super::extract::ApiJson;
use crate::{
    app::AppState,
    audit::{self, Entry},
    error::{ApiError, ApiResult, ErrorBody},
    events::{self, Event, kind},
    httpserver::Authn,
    rbac::{Role, Scope},
};

#[derive(Serialize, ToSchema)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

macro_rules! project_row {
    ($r:expr) => {
        Project {
            id: $r.id,
            name: $r.name,
            slug: $r.slug,
            description: $r.description,
            created_at: $r.created_at,
            updated_at: $r.updated_at,
        }
    };
}

fn valid_slug(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 40
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && b[0] != b'-'
        && b[b.len() - 1] != b'-'
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}

#[utoipa::path(
    operation_id = "projects_list",
    get,
    path = "/api/v1/projects",
    tag = "projects",
    responses((status = 200, description = "Projects the caller can see", body = Vec<Project>)),
    extensions(("x-permission" = json!({"action": "project.list", "scope": "self"}))),
)]
pub async fn list(State(app): State<AppState>, Extension(me): Extension<Authn>) -> ApiResult<Json<Vec<Project>>> {
    let ids: Vec<Uuid> = me.grants.scoped.keys().filter(|(s, _)| *s == Scope::Project).map(|(_, id)| *id).collect();
    let see_all = me.grants.global.is_some();
    let rows = sqlx::query!(
        r#"select id, name, slug, description, created_at as "created_at: DateTime<Utc>", updated_at as "updated_at: DateTime<Utc>"
             from projects where $1 or id = any($2) order by name"#,
        see_all,
        &ids
    )
    .fetch_all(&app.pool)
    .await?;
    Ok(Json(rows.into_iter().map(|r| project_row!(r)).collect()))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateProject {
    pub name: String,
    /// Lowercase letters, digits and hyphens, up to 40 characters.
    pub slug: String,
    pub description: Option<String>,
}

#[utoipa::path(
    operation_id = "projects_create",
    post,
    path = "/api/v1/projects",
    tag = "projects",
    request_body = CreateProject,
    responses((status = 201, body = Project), (status = 400, body = ErrorBody), (status = 409, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "project.create", "scope": "global"}))),
)]
pub async fn create(
    State(app): State<AppState>,
    Extension(me): Extension<Authn>,
    ApiJson(req): ApiJson<CreateProject>,
) -> ApiResult<(StatusCode, Json<Project>)> {
    let name = req.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request("name must be 1 to 100 characters"));
    }
    if !valid_slug(&req.slug) {
        return Err(ApiError::bad_request("slug must be lowercase letters, digits and hyphens, not starting or ending with a hyphen"));
    }
    let mut tx = app.pool.begin().await?;
    let row = sqlx::query!(
        r#"insert into projects (name, slug, description, created_by) values ($1, $2, $3, $4)
           returning id, name, slug, description, created_at as "created_at: DateTime<Utc>", updated_at as "updated_at: DateTime<Utc>""#,
        name,
        req.slug,
        req.description.unwrap_or_default(),
        me.user_id
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| if is_unique_violation(&e) { ApiError::conflict("a project with that slug already exists") } else { e.into() })?;
    audit::record(
        &mut tx,
        &me.actor,
        &Entry::new("project.create", "global").resource("project", row.id).meta(json!({ "slug": row.slug })),
    )
    .await?;
    events::emit(&mut tx, Event { kind: kind::PROJECT_CREATED, project_id: Some(row.id), data: json!({ "slug": row.slug }) }).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(project_row!(row))))
}

#[utoipa::path(
    operation_id = "projects_get",
    get,
    path = "/api/v1/projects/{project_id}",
    tag = "projects",
    params(("project_id" = Uuid, Path, description = "Project id")),
    responses((status = 200, body = Project), (status = 404, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "project.read", "scope": "project"}))),
)]
pub async fn get(State(app): State<AppState>, Path(project_id): Path<Uuid>) -> ApiResult<Json<Project>> {
    let row = sqlx::query!(
        r#"select id, name, slug, description, created_at as "created_at: DateTime<Utc>", updated_at as "updated_at: DateTime<Utc>"
             from projects where id = $1"#,
        project_id
    )
    .fetch_optional(&app.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("no such project"))?;
    Ok(Json(project_row!(row)))
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateProject {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[utoipa::path(
    operation_id = "projects_update",
    patch,
    path = "/api/v1/projects/{project_id}",
    tag = "projects",
    params(("project_id" = Uuid, Path, description = "Project id")),
    request_body = UpdateProject,
    responses((status = 200, body = Project), (status = 404, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "project.update", "scope": "project"}))),
)]
pub async fn update(
    State(app): State<AppState>,
    Extension(me): Extension<Authn>,
    Path(project_id): Path<Uuid>,
    ApiJson(req): ApiJson<UpdateProject>,
) -> ApiResult<Json<Project>> {
    if let Some(n) = &req.name
        && (n.trim().is_empty() || n.chars().count() > 100)
    {
        return Err(ApiError::bad_request("name must be 1 to 100 characters"));
    }
    let mut tx = app.pool.begin().await?;
    let row = sqlx::query!(
        r#"update projects set name = coalesce($2, name), description = coalesce($3, description), updated_at = now()
            where id = $1
        returning id, name, slug, description, created_at as "created_at: DateTime<Utc>", updated_at as "updated_at: DateTime<Utc>""#,
        project_id,
        req.name.map(|n| n.trim().to_owned()),
        req.description
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("no such project"))?;
    audit::record(&mut tx, &me.actor, &Entry::new("project.update", "project").scope_id(Some(project_id)).resource("project", project_id))
        .await?;
    tx.commit().await?;
    Ok(Json(project_row!(row)))
}

#[utoipa::path(
    operation_id = "projects_delete",
    delete,
    path = "/api/v1/projects/{project_id}",
    tag = "projects",
    params(("project_id" = Uuid, Path, description = "Project id")),
    responses((status = 204, description = "Deleted"), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "project.delete", "scope": "project", "reauth": true}))),
)]
pub async fn delete(State(app): State<AppState>, Extension(me): Extension<Authn>, Path(project_id): Path<Uuid>) -> ApiResult<StatusCode> {
    let mut tx = app.pool.begin().await?;
    let slug = sqlx::query_scalar!("select slug from projects where id = $1 for update", project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("no such project"))?;
    sqlx::query!("delete from memberships where scope = 'project' and scope_id = $1", project_id).execute(&mut *tx).await?;
    revoke_token_access(&mut tx, project_id, None).await?;
    sqlx::query!("delete from projects where id = $1", project_id).execute(&mut *tx).await?;
    audit::record(
        &mut tx,
        &me.actor,
        &Entry::new("project.delete", "project").scope_id(Some(project_id)).resource("project", project_id).meta(json!({ "slug": slug })),
    )
    .await?;
    events::emit(&mut tx, Event { kind: kind::PROJECT_DELETED, project_id: Some(project_id), data: json!({ "slug": slug }) }).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a project from API tokens, optionally only those of one user. A token left with no
/// project is revoked, since it could no longer do anything useful.
async fn revoke_token_access(tx: &mut sqlx::PgConnection, project_id: Uuid, user_id: Option<Uuid>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update api_tokens set revoked_at = now()
          where $1 = any(project_ids) and cardinality(project_ids) = 1 and revoked_at is null and ($2::uuid is null or user_id = $2)",
        project_id,
        user_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "update api_tokens set project_ids = array_remove(project_ids, $1)
          where $1 = any(project_ids) and cardinality(project_ids) > 1 and ($2::uuid is null or user_id = $2)",
        project_id,
        user_id
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

#[derive(Serialize, ToSchema)]
pub struct Member {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
}

#[utoipa::path(
    operation_id = "projects_members",
    get,
    path = "/api/v1/projects/{project_id}/members",
    tag = "projects",
    params(("project_id" = Uuid, Path, description = "Project id")),
    responses((status = 200, body = Vec<Member>)),
    extensions(("x-permission" = json!({"action": "project.member.list", "scope": "project"}))),
)]
pub async fn members(State(app): State<AppState>, Path(project_id): Path<Uuid>) -> ApiResult<Json<Vec<Member>>> {
    let rows = sqlx::query!(
        "select u.id, u.email, u.display_name, m.role from memberships m join users u on u.id = m.user_id
          where m.scope = 'project' and m.scope_id = $1 order by u.email",
        project_id
    )
    .fetch_all(&app.pool)
    .await?;
    Ok(Json(rows.into_iter().map(|r| Member { user_id: r.id, email: r.email, display_name: r.display_name, role: r.role }).collect()))
}

#[derive(Deserialize, ToSchema)]
pub struct SetMember {
    /// `admin`, `developer` or `viewer`. Owner is an installation-wide role and cannot be set here.
    pub role: String,
}

#[utoipa::path(
    operation_id = "projects_set_member",
    put,
    path = "/api/v1/projects/{project_id}/members/{user_id}",
    tag = "projects",
    params(("project_id" = Uuid, Path, description = "Project id"), ("user_id" = Uuid, Path, description = "User to add or change")),
    request_body = SetMember,
    responses((status = 204, description = "Role set"), (status = 404, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "project.member.set", "scope": "project"}))),
)]
pub async fn set_member(
    State(app): State<AppState>,
    Extension(me): Extension<Authn>,
    Path((project_id, user_id)): Path<(Uuid, Uuid)>,
    ApiJson(req): ApiJson<SetMember>,
) -> ApiResult<StatusCode> {
    let role = Role::parse(&req.role)
        .filter(|r| *r != Role::Owner)
        .ok_or_else(|| ApiError::bad_request("role must be admin, developer or viewer"))?;
    let mut tx = app.pool.begin().await?;
    let exists = sqlx::query_scalar!(r#"select exists(select 1 from users where id = $1 and disabled_at is null) as "e!""#, user_id)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Err(ApiError::not_found("no such user"));
    }
    sqlx::query!(
        "insert into memberships (user_id, scope, scope_id, role) values ($1, 'project', $2, $3)
         on conflict (user_id, scope, coalesce(scope_id, '00000000-0000-0000-0000-000000000000'::uuid)) do update set role = excluded.role",
        user_id,
        project_id,
        role.as_str()
    )
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        &me.actor,
        &Entry::new("project.member.set", "project")
            .scope_id(Some(project_id))
            .resource("user", user_id)
            .meta(json!({ "role": role.as_str() })),
    )
    .await?;
    events::emit(
        &mut tx,
        Event { kind: kind::MEMBER_SET, project_id: Some(project_id), data: json!({ "user_id": user_id, "role": role.as_str() }) },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    operation_id = "projects_remove_member",
    delete,
    path = "/api/v1/projects/{project_id}/members/{user_id}",
    tag = "projects",
    params(("project_id" = Uuid, Path, description = "Project id"), ("user_id" = Uuid, Path, description = "User to remove")),
    responses((status = 204, description = "Removed"), (status = 404, body = ErrorBody)),
    extensions(("x-permission" = json!({"action": "project.member.remove", "scope": "project"}))),
)]
pub async fn remove_member(
    State(app): State<AppState>,
    Extension(me): Extension<Authn>,
    Path((project_id, user_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let mut tx = app.pool.begin().await?;
    let removed = sqlx::query!("delete from memberships where scope = 'project' and scope_id = $1 and user_id = $2", project_id, user_id)
        .execute(&mut *tx)
        .await?;
    if removed.rows_affected() == 0 {
        return Err(ApiError::not_found("that user is not a member"));
    }
    // Their tokens lose the project immediately instead of waiting for the next role check.
    revoke_token_access(&mut tx, project_id, Some(user_id)).await?;
    audit::record(&mut tx, &me.actor, &Entry::new("project.member.remove", "project").scope_id(Some(project_id)).resource("user", user_id))
        .await?;
    events::emit(&mut tx, Event { kind: kind::MEMBER_REMOVED, project_id: Some(project_id), data: json!({ "user_id": user_id }) }).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
