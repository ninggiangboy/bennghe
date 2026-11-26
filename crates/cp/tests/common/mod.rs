// SPDX-License-Identifier: AGPL-3.0-only
//! Shared harness: a real router on a fresh database per test, plus a cookie-aware client.
#![allow(dead_code)]
use std::{path::PathBuf, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use bn_cp::{
    app,
    auth::password,
    config::Config,
    jobs,
    secrets::{Identity, Secrets, SigningKeyFile},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

pub struct App {
    pub router: Router,
    pub pool: PgPool,
}

pub async fn app(pool: PgPool) -> App {
    app_with(pool, |_| {}).await
}

pub async fn app_with(pool: PgPool, tweak: impl FnOnce(&mut Config)) -> App {
    let dir: PathBuf = std::env::temp_dir().join(format!("bn-it-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    Secrets::generate_file(&dir.join("master.key")).unwrap();
    SigningKeyFile::generate(&dir.join("signing.key")).unwrap();
    let secrets = Secrets::load(&dir.join("master.key")).unwrap();
    secrets.verify_against(&pool).await.unwrap();
    let identity = Identity::ensure(&pool, &dir.join("signing.key")).await.unwrap();
    jobs::init(&pool).await.unwrap();
    let mut config = Config::for_tests(dir);
    tweak(&mut config);
    let (router, _) = app::build(pool.clone(), Arc::new(secrets), Arc::new(config), Arc::new(identity)).unwrap();
    App { router, pool }
}

pub struct Resp {
    pub status: StatusCode,
    pub body: Value,
    pub set_cookie: Vec<String>,
}

impl Resp {
    pub fn code(&self) -> &str {
        self.body["code"].as_str().unwrap_or("")
    }
}

pub struct Client {
    router: Router,
    pub cookie: Option<String>,
    pub csrf: Option<String>,
    pub bearer: Option<String>,
}

impl Client {
    pub fn new(app: &App) -> Self {
        Self { router: app.router.clone(), cookie: None, csrf: None, bearer: None }
    }

    pub fn with_bearer(app: &App, token: &str) -> Self {
        Self { bearer: Some(token.to_owned()), ..Self::new(app) }
    }

    pub async fn send(&mut self, method: Method, path: &str, body: Option<Value>) -> Resp {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(c) = &self.cookie {
            req = req.header(header::COOKIE, c);
        }
        if let Some(c) = &self.csrf {
            req = req.header("x-csrf-token", c);
        }
        if let Some(b) = &self.bearer {
            req = req.header(header::AUTHORIZATION, format!("Bearer {b}"));
        }
        let req = match body {
            Some(v) => req.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let resp = self.router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let set_cookie: Vec<String> = resp.headers().get_all(header::SET_COOKIE).iter().map(|v| v.to_str().unwrap().to_owned()).collect();
        for sc in &set_cookie {
            let pair = sc.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            if value.is_empty() {
                self.cookie = None;
            } else if name.ends_with("bn_sid") {
                self.cookie = Some(pair.to_owned());
            }
        }
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if let Some(t) = body.get("csrf_token").and_then(Value::as_str) {
            self.csrf = Some(t.to_owned());
        }
        Resp { status, body, set_cookie }
    }

    pub async fn get(&mut self, path: &str) -> Resp {
        self.send(Method::GET, path, None).await
    }
    pub async fn post(&mut self, path: &str, body: Value) -> Resp {
        self.send(Method::POST, path, Some(body)).await
    }
    pub async fn put(&mut self, path: &str, body: Value) -> Resp {
        self.send(Method::PUT, path, Some(body)).await
    }
    pub async fn patch(&mut self, path: &str, body: Value) -> Resp {
        self.send(Method::PATCH, path, Some(body)).await
    }
    pub async fn delete(&mut self, path: &str) -> Resp {
        self.send(Method::DELETE, path, None).await
    }

    /// Signs in with a password and expects no second factor.
    pub async fn login(&mut self, email: &str, password: &str) -> Resp {
        let r = self.post("/api/v1/auth/login", json!({ "email": email, "password": password })).await;
        assert_eq!(r.status, StatusCode::OK, "login failed: {:?}", r.body);
        r
    }
}

pub const PASSWORD: &str = "correct horse battery staple";

pub async fn make_user(pool: &PgPool, email: &str, global_role: Option<&str>) -> Uuid {
    let hash = password::hash_blocking(PASSWORD).unwrap();
    let id = sqlx::query_scalar!("insert into users (email, password_hash) values ($1, $2) returning id", email, hash)
        .fetch_one(pool)
        .await
        .unwrap();
    if let Some(role) = global_role {
        sqlx::query!("insert into memberships (user_id, scope, role) values ($1, 'global', $2)", id, role).execute(pool).await.unwrap();
    }
    id
}

pub async fn make_project(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar!("insert into projects (name, slug) values ($1, $1) returning id", slug).fetch_one(pool).await.unwrap()
}

pub async fn add_member(pool: &PgPool, user: Uuid, project: Uuid, role: &str) {
    sqlx::query!(
        "insert into memberships (user_id, scope, scope_id, role) values ($1, 'project', $2, $3)
         on conflict (user_id, scope, coalesce(scope_id, '00000000-0000-0000-0000-000000000000'::uuid)) do update set role = excluded.role",
        user,
        project,
        role
    )
    .execute(pool)
    .await
    .unwrap();
}

/// A signed-in client for a freshly created user.
pub async fn signed_in(app: &App, email: &str, global_role: Option<&str>) -> (Client, Uuid) {
    let id = make_user(&app.pool, email, global_role).await;
    let mut c = Client::new(app);
    c.login(email, PASSWORD).await;
    (c, id)
}

pub async fn denied_count(pool: &PgPool, action: &str, reason: &str) -> i64 {
    sqlx::query_scalar!(
        r#"select count(*) as "n!" from audit_events where outcome = 'denied' and action = $1 and reason = $2"#,
        action,
        reason
    )
    .fetch_one(pool)
    .await
    .unwrap()
}
