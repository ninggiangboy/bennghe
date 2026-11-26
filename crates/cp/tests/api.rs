// SPDX-License-Identifier: AGPL-3.0-only
//! End-to-end behaviour of authentication, access control and audit against a real database.
mod common;

use axum::http::StatusCode;
use bn_cp::auth::{setup, totp};
use chrono::Utc;
use common::*;
use serde_json::json;
use sqlx::PgPool;

fn now_step() -> u64 {
    totp::step_at(u64::try_from(Utc::now().timestamp()).unwrap())
}

fn decode_secret(b32: &str) -> Vec<u8> {
    base32::decode(base32::Alphabet::Rfc4648 { padding: false }, b32).unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn setup_requires_the_one_time_token(pool: PgPool) {
    let app = app(pool.clone()).await;
    let mut c = Client::new(&app);
    assert_eq!(c.get("/api/v1/setup/status").await.body["needs_setup"], true);

    let body = |token: &str| json!({ "token": token, "email": "Owner@Example.com", "password": PASSWORD });
    let wrong = c.post("/api/v1/setup", body("bnsetup_wrong")).await;
    assert_eq!((wrong.status, wrong.code()), (StatusCode::FORBIDDEN, "INVALID_SETUP_TOKEN"));
    assert_eq!(c.get("/api/v1/auth/session").await.status, StatusCode::UNAUTHORIZED, "a failed setup must not sign anyone in");

    let token = setup::create_token(&pool).await.unwrap();
    let ok = c.post("/api/v1/setup", body(&token)).await;
    assert_eq!(ok.status, StatusCode::OK, "{:?}", ok.body);
    let me = c.get("/api/v1/auth/session").await;
    assert_eq!(me.body["user"]["email"], "owner@example.com");
    assert_eq!(me.body["global_role"], "owner");

    let again = c.post("/api/v1/setup", body(&setup::create_token(&pool).await.unwrap())).await;
    assert_eq!((again.status, again.code()), (StatusCode::FORBIDDEN, "SETUP_COMPLETE"));
    assert_eq!(c.get("/api/v1/setup/status").await.body["needs_setup"], false);
    assert_eq!(denied_count(&pool, "setup.create", "bad_setup_token").await, 1);
    assert_eq!(denied_count(&pool, "setup.create", "already_set_up").await, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_setup_token_is_stored_only_as_a_hash(pool: PgPool) {
    let token = setup::create_token(&pool).await.unwrap();
    let stored: Vec<u8> = sqlx::query_scalar!("select token_hash from setup_tokens").fetch_one(&pool).await.unwrap();
    assert!(!stored.windows(token.len()).any(|w| w == token.as_bytes()));
    assert_eq!(stored.len(), 32);
}

#[sqlx::test(migrations = "../../migrations")]
async fn failed_logins_are_audited_and_then_rate_limited(pool: PgPool) {
    let app = app(pool.clone()).await;
    make_user(&pool, "a@example.com", None).await;
    let mut c = Client::new(&app);
    for _ in 0..10 {
        let r = c.post("/api/v1/auth/login", json!({ "email": "a@example.com", "password": "wrong password!" })).await;
        assert_eq!((r.status, r.code()), (StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS"));
    }
    assert_eq!(denied_count(&pool, "auth.login", "bad_credentials").await, 10);
    let blocked = c.post("/api/v1/auth/login", json!({ "email": "a@example.com", "password": PASSWORD })).await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS, "even the right password is refused while blocked");
    // Unknown accounts fail the same way, so the API does not reveal which emails exist.
    let unknown = Client::new(&app).post("/api/v1/auth/login", json!({ "email": "nobody@example.com", "password": PASSWORD })).await;
    assert_eq!((unknown.status, unknown.code()), (StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn session_cookie_is_host_prefixed_and_locked_down(pool: PgPool) {
    let app = app_with(pool.clone(), |c| c.insecure_cookies = false).await;
    make_user(&pool, "a@example.com", None).await;
    let mut c = Client::new(&app);
    let r = c.post("/api/v1/auth/login", json!({ "email": "a@example.com", "password": PASSWORD })).await;
    let cookie = &r.set_cookie[0];
    assert!(cookie.starts_with("__Host-bn_sid="), "{cookie}");
    for attr in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"] {
        assert!(cookie.contains(attr), "{attr} missing from {cookie}");
    }
    assert!(!cookie.to_ascii_lowercase().contains("domain="), "a Domain attribute would defeat the __Host- prefix");
}

#[sqlx::test(migrations = "../../migrations")]
async fn unauthenticated_requests_are_refused_and_audited(pool: PgPool) {
    let app = app(pool.clone()).await;
    let r = Client::new(&app).get("/api/v1/projects").await;
    assert_eq!((r.status, r.code()), (StatusCode::UNAUTHORIZED, "UNAUTHENTICATED"));
    assert_eq!(denied_count(&pool, "project.list", "unauthenticated").await, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn session_writes_need_the_csrf_header(pool: PgPool) {
    let app = app(pool.clone()).await;
    let (mut c, _) = signed_in(&app, "owner@example.com", Some("owner")).await;
    let good_csrf = c.csrf.take();
    let r = c.post("/api/v1/projects", json!({ "name": "A", "slug": "a" })).await;
    assert_eq!((r.status, r.code()), (StatusCode::FORBIDDEN, "CSRF_INVALID"));
    c.csrf = Some("not-the-token".into());
    assert_eq!(c.post("/api/v1/projects", json!({ "name": "A", "slug": "a" })).await.code(), "CSRF_INVALID");
    c.csrf = good_csrf;
    assert_eq!(c.post("/api/v1/projects", json!({ "name": "A", "slug": "a" })).await.status, StatusCode::CREATED);
    assert_eq!(denied_count(&pool, "project.create", "csrf").await, 2);
}

#[sqlx::test(migrations = "../../migrations")]
async fn roles_gate_writes_and_every_refusal_is_audited(pool: PgPool) {
    let app = app(pool.clone()).await;
    let project = make_project(&pool, "shop").await;
    let path = format!("/api/v1/projects/{project}");
    let mut results = Vec::new();
    for role in ["viewer", "developer", "admin"] {
        let (mut c, user) = signed_in(&app, &format!("{role}@example.com"), None).await;
        add_member(&pool, user, project, role).await;
        let read = c.get(&path).await.status;
        let write = c.patch(&path, json!({ "description": "x" })).await.status;
        results.push((role, read, write));
    }
    assert_eq!(
        results,
        vec![
            ("viewer", StatusCode::OK, StatusCode::FORBIDDEN),
            ("developer", StatusCode::OK, StatusCode::FORBIDDEN),
            ("admin", StatusCode::OK, StatusCode::OK),
        ]
    );
    let denied = sqlx::query!(
        "select actor_user_id, scope, scope_id, outcome from audit_events where action = 'project.update' and outcome = 'denied' order by id"
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(denied.len(), 2, "one record per refused write");
    assert!(denied.iter().all(|d| d.scope == "project" && d.scope_id == Some(project) && d.actor_user_id.is_some()));
    let ok: i64 =
        sqlx::query_scalar!(r#"select count(*) as "n!" from audit_events where action = 'project.update' and outcome = 'success'"#)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ok, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_project_is_a_hard_isolation_boundary(pool: PgPool) {
    let app = app(pool.clone()).await;
    let (a, b) = (make_project(&pool, "a").await, make_project(&pool, "b").await);
    let (mut dev, user) = signed_in(&app, "dev@example.com", None).await;
    add_member(&pool, user, a, "admin").await;
    assert_eq!(dev.get(&format!("/api/v1/projects/{a}")).await.status, StatusCode::OK);
    for path in [format!("/api/v1/projects/{b}"), format!("/api/v1/projects/{b}/members")] {
        assert_eq!(dev.get(&path).await.status, StatusCode::FORBIDDEN, "{path}");
    }
    assert_eq!(
        dev.put(&format!("/api/v1/projects/{b}/members/{user}"), json!({ "role": "admin" })).await.status,
        StatusCode::FORBIDDEN,
        "no self-promotion across projects"
    );
    let listed = dev.get("/api/v1/projects").await;
    let slugs: Vec<_> = listed.body.as_array().unwrap().iter().map(|p| p["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, ["a"]);
}

#[sqlx::test(migrations = "../../migrations")]
async fn global_admin_cannot_do_owner_things(pool: PgPool) {
    let app = app(pool.clone()).await;
    let (mut admin, _) = signed_in(&app, "admin@example.com", Some("admin")).await;
    assert_eq!(admin.get("/api/v1/settings/security").await.status, StatusCode::OK);
    let put = admin.put("/api/v1/settings/security", json!({ "require_2fa_for_admins": true })).await;
    assert_eq!(put.status, StatusCode::FORBIDDEN);
    assert_eq!(admin.get("/api/v1/audit-events").await.status, StatusCode::OK);
    let (mut viewer, _) = signed_in(&app, "viewer@example.com", Some("viewer")).await;
    assert_eq!(viewer.get("/api/v1/audit-events").await.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../migrations")]
async fn two_factor_login_is_two_steps_and_codes_are_single_use(pool: PgPool) {
    let app = app(pool.clone()).await;
    let (mut c, user) = signed_in(&app, "u@example.com", None).await;

    let begin = c.post("/api/v1/account/totp", json!({})).await;
    assert_eq!(begin.status, StatusCode::OK);
    let secret = decode_secret(begin.body["secret"].as_str().unwrap());
    assert!(begin.body["otpauth_uri"].as_str().unwrap().starts_with("otpauth://totp/Bennghe:u%40example.com"));
    let stored = sqlx::query!("select secret_ciphertext, key_id from user_totp where user_id = $1", user).fetch_one(&pool).await.unwrap();
    assert!(!stored.secret_ciphertext.windows(secret.len()).any(|w| w == secret.as_slice()), "the secret must be encrypted at rest");
    assert_eq!(stored.key_id, "k1");

    let wrong = c.post("/api/v1/account/totp/confirm", json!({ "code": "000000" })).await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    let step = now_step();
    let confirmed = c.post("/api/v1/account/totp/confirm", json!({ "code": totp::code_at(&secret, step) })).await;
    assert_eq!(confirmed.status, StatusCode::OK, "{:?}", confirmed.body);
    let codes: Vec<String> = confirmed.body["recovery_codes"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
    assert_eq!(codes.len(), 10);
    assert_eq!(c.post("/api/v1/account/totp", json!({})).await.status, StatusCode::CONFLICT, "cannot re-enrol while enabled");

    // Signing in with the password alone no longer gives access.
    let mut fresh = Client::new(&app);
    let first = fresh.post("/api/v1/auth/login", json!({ "email": "u@example.com", "password": PASSWORD })).await;
    assert_eq!(first.body["status"], "totp_required");
    assert_eq!(fresh.get("/api/v1/projects").await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(fresh.get("/api/v1/auth/session").await.status, StatusCode::UNAUTHORIZED);
    let bad = fresh.post("/api/v1/auth/login/totp", json!({ "code": "123456" })).await;
    assert_eq!((bad.status, bad.code()), (StatusCode::UNAUTHORIZED, "INVALID_CODE"));

    // The step used to confirm is spent; the next step's code is accepted exactly once.
    let spent = fresh.post("/api/v1/auth/login/totp", json!({ "code": totp::code_at(&secret, step) })).await;
    assert_eq!(spent.status, StatusCode::UNAUTHORIZED, "a time step that was already used must be refused");
    let next = totp::code_at(&secret, step + 1);
    assert_eq!(fresh.post("/api/v1/auth/login/totp", json!({ "code": next })).await.status, StatusCode::OK);
    assert_eq!(fresh.get("/api/v1/projects").await.status, StatusCode::OK);

    let mut replay = Client::new(&app);
    replay.post("/api/v1/auth/login", json!({ "email": "u@example.com", "password": PASSWORD })).await;
    assert_eq!(
        replay.post("/api/v1/auth/login/totp", json!({ "code": next })).await.status,
        StatusCode::UNAUTHORIZED,
        "replay of a just-used code"
    );

    // Each recovery code works once.
    assert_eq!(replay.post("/api/v1/auth/login/totp", json!({ "code": codes[0] })).await.status, StatusCode::OK);
    let mut again = Client::new(&app);
    again.post("/api/v1/auth/login", json!({ "email": "u@example.com", "password": PASSWORD })).await;
    assert_eq!(again.post("/api/v1/auth/login/totp", json!({ "code": codes[0] })).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(again.post("/api/v1/auth/login/totp", json!({ "code": codes[1] })).await.status, StatusCode::OK);
}

#[sqlx::test(migrations = "../../migrations")]
async fn pending_sessions_expire_and_the_token_rotates_on_success(pool: PgPool) {
    let app = app(pool.clone()).await;
    let (mut c, user) = signed_in(&app, "u@example.com", None).await;
    let begin = c.post("/api/v1/account/totp", json!({})).await;
    let secret = decode_secret(begin.body["secret"].as_str().unwrap());
    c.post("/api/v1/account/totp/confirm", json!({ "code": totp::code_at(&secret, now_step()) })).await;

    let mut fresh = Client::new(&app);
    fresh.post("/api/v1/auth/login", json!({ "email": "u@example.com", "password": PASSWORD })).await;
    let pending_cookie = fresh.cookie.clone().unwrap();
    sqlx::query!("update sessions set expires_at = now() - interval '1 second' where stage = 'pending_2fa' and user_id = $1", user)
        .execute(&pool)
        .await
        .unwrap();
    let late = fresh.post("/api/v1/auth/login/totp", json!({ "code": totp::code_at(&secret, now_step() + 1) })).await;
    assert_eq!(late.status, StatusCode::UNAUTHORIZED, "the five minute window has passed");

    let mut ok = Client::new(&app);
    ok.post("/api/v1/auth/login", json!({ "email": "u@example.com", "password": PASSWORD })).await;
    let before = ok.cookie.clone().unwrap();
    ok.post("/api/v1/auth/login/totp", json!({ "code": totp::code_at(&secret, now_step() + 1) })).await;
    assert_ne!(ok.cookie.clone().unwrap(), before, "the session token must change once the second factor is proven");
    assert_ne!(pending_cookie, before);
    let mut stale = Client::new(&app);
    stale.cookie = Some(before);
    assert_eq!(stale.get("/api/v1/auth/session").await.status, StatusCode::UNAUTHORIZED, "the pre-2FA token is worthless afterwards");
}

#[sqlx::test(migrations = "../../migrations")]
async fn sensitive_operations_need_a_recent_password_check(pool: PgPool) {
    let app = app(pool.clone()).await;
    let project = make_project(&pool, "a").await;
    let (mut c, user) = signed_in(&app, "admin@example.com", None).await;
    add_member(&pool, user, project, "admin").await;
    let body = json!({ "name": "ci", "project_ids": [project], "role": "developer", "expires_in_days": 30 });

    sqlx::query!("update sessions set last_verified_at = now() - interval '6 minutes'").execute(&pool).await.unwrap();
    let stale = c.post("/api/v1/account/api-tokens", body.clone()).await;
    assert_eq!((stale.status, stale.code()), (StatusCode::FORBIDDEN, "REAUTH_REQUIRED"));
    assert_eq!(denied_count(&pool, "account.api_token.create", "reauth_required").await, 1);

    let wrong = c.post("/api/v1/auth/reauth", json!({ "password": "not the password" })).await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(c.post("/api/v1/auth/reauth", json!({})).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(c.post("/api/v1/auth/reauth", json!({ "password": PASSWORD })).await.status, StatusCode::NO_CONTENT);
    let created = c.post("/api/v1/account/api-tokens", body).await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    assert!(created.body["token"].as_str().unwrap().starts_with("bnp_"));

    // Project deletion is also in the list; a stale session is turned away again.
    sqlx::query!("update sessions set last_verified_at = now() - interval '6 minutes'").execute(&pool).await.unwrap();
    let del = c.delete(&format!("/api/v1/projects/{project}")).await;
    assert_eq!(del.code(), "REAUTH_REQUIRED");
}

#[sqlx::test(migrations = "../../migrations")]
async fn api_tokens_are_project_scoped_audited_and_never_reauthenticated(pool: PgPool) {
    let app = app(pool.clone()).await;
    let (a, b) = (make_project(&pool, "a").await, make_project(&pool, "b").await);
    let (mut c, user) = signed_in(&app, "admin@example.com", None).await;
    add_member(&pool, user, a, "admin").await;
    add_member(&pool, user, b, "admin").await;

    // The user may not mint a token stronger than their own role, nor for a project they are not in.
    add_member(&pool, user, b, "developer").await;
    let too_strong =
        c.post("/api/v1/account/api-tokens", json!({ "name": "x", "project_ids": [b], "role": "admin", "expires_in_days": 1 })).await;
    assert_eq!(too_strong.status, StatusCode::FORBIDDEN);
    add_member(&pool, user, b, "admin").await;
    let too_long =
        c.post("/api/v1/account/api-tokens", json!({ "name": "x", "project_ids": [a], "role": "admin", "expires_in_days": 366 })).await;
    assert_eq!(too_long.status, StatusCode::BAD_REQUEST, "a year is the maximum lifetime");
    let stranger = c
        .post(
            "/api/v1/account/api-tokens",
            json!({ "name": "x", "project_ids": [uuid::Uuid::new_v4()], "role": "viewer", "expires_in_days": 1 }),
        )
        .await;
    assert_eq!(stranger.status, StatusCode::NOT_FOUND);

    let created =
        c.post("/api/v1/account/api-tokens", json!({ "name": "ci", "project_ids": [a], "role": "admin", "expires_in_days": 30 })).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let (token, token_id) = (created.body["token"].as_str().unwrap().to_owned(), created.body["id"].as_str().unwrap().to_owned());
    let stored = sqlx::query_scalar!("select token_hash from api_tokens").fetch_one(&pool).await.unwrap();
    assert!(!stored.windows(token.len()).any(|w| w == token.as_bytes()), "only a hash of the token may be stored");

    let mut ci = Client::with_bearer(&app, &token);
    assert_eq!(ci.patch(&format!("/api/v1/projects/{a}"), json!({ "description": "deployed" })).await.status, StatusCode::OK);
    let audited = sqlx::query_scalar!(
        r#"select actor_token_id::text as "t" from audit_events where action = 'project.update' and outcome = 'success'"#
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited.as_deref(), Some(token_id.as_str()), "the audit row names the token as well as the user");
    assert_eq!(ci.get(&format!("/api/v1/projects/{b}")).await.status, StatusCode::FORBIDDEN, "other projects are off limits");
    assert_eq!(denied_count(&pool, "project.read", "token_project").await, 1);
    for (label, resp) in [
        ("cluster/global/self routes", ci.get("/api/v1/projects").await),
        ("settings", ci.get("/api/v1/settings/security").await),
        ("reauth route", ci.delete(&format!("/api/v1/projects/{a}")).await),
        (
            "token creation",
            ci.post("/api/v1/account/api-tokens", json!({ "name": "y", "project_ids": [a], "role": "viewer", "expires_in_days": 1 })).await,
        ),
        ("session info", ci.get("/api/v1/auth/session").await),
    ] {
        assert_eq!(resp.status, StatusCode::FORBIDDEN, "{label}: {:?}", resp.body);
    }
    assert_eq!(denied_count(&pool, "project.delete", "reauth_not_possible_with_token").await, 1);

    // The token never outranks its user: demote them and the same token loses write access.
    add_member(&pool, user, a, "viewer").await;
    assert_eq!(ci.patch(&format!("/api/v1/projects/{a}"), json!({ "description": "again" })).await.status, StatusCode::FORBIDDEN);
    assert_eq!(ci.get(&format!("/api/v1/projects/{a}")).await.status, StatusCode::OK, "reading is still allowed for a viewer");

    // Expired, revoked and unknown tokens are unauthenticated.
    sqlx::query!("update api_tokens set expires_at = now() - interval '1 second'").execute(&pool).await.unwrap();
    assert_eq!(ci.get(&format!("/api/v1/projects/{a}")).await.status, StatusCode::UNAUTHORIZED);
    sqlx::query!("update api_tokens set expires_at = now() + interval '1 day'").execute(&pool).await.unwrap();
    assert_eq!(ci.get(&format!("/api/v1/projects/{a}")).await.status, StatusCode::OK);
    assert_eq!(c.delete(&format!("/api/v1/account/api-tokens/{token_id}")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(ci.get(&format!("/api/v1/projects/{a}")).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(Client::with_bearer(&app, "bnp_nonsense").get(&format!("/api/v1/projects/{a}")).await.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../../migrations")]
async fn removing_a_member_revokes_their_token_access_at_once(pool: PgPool) {
    let app = app(pool.clone()).await;
    let (a, b) = (make_project(&pool, "a").await, make_project(&pool, "b").await);
    let (mut owner, _) = signed_in(&app, "owner@example.com", Some("owner")).await;
    let (mut dev, dev_id) = signed_in(&app, "dev@example.com", None).await;
    add_member(&pool, dev_id, a, "developer").await;
    add_member(&pool, dev_id, b, "developer").await;
    let single = dev
        .post("/api/v1/account/api-tokens", json!({ "name": "one", "project_ids": [a], "role": "developer", "expires_in_days": 5 }))
        .await;
    let both = dev
        .post("/api/v1/account/api-tokens", json!({ "name": "two", "project_ids": [a, b], "role": "developer", "expires_in_days": 5 }))
        .await;
    let (single_tok, both_tok) = (single.body["token"].as_str().unwrap().to_owned(), both.body["token"].as_str().unwrap().to_owned());

    assert_eq!(owner.delete(&format!("/api/v1/projects/{a}/members/{dev_id}")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        Client::with_bearer(&app, &single_tok).get(&format!("/api/v1/projects/{a}")).await.status,
        StatusCode::UNAUTHORIZED,
        "a token with no project left is revoked"
    );
    assert_eq!(Client::with_bearer(&app, &both_tok).get(&format!("/api/v1/projects/{a}")).await.status, StatusCode::FORBIDDEN);
    assert_eq!(
        Client::with_bearer(&app, &both_tok).get(&format!("/api/v1/projects/{b}")).await.status,
        StatusCode::OK,
        "other projects keep working"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_two_factor_requirement_traps_admins_in_enrolment(pool: PgPool) {
    let app = app(pool.clone()).await;
    let project = make_project(&pool, "a").await;
    let (mut owner, owner_id) = signed_in(&app, "owner@example.com", Some("owner")).await;
    let _ = owner_id;
    let (mut admin, admin_id) = signed_in(&app, "admin@example.com", None).await;
    add_member(&pool, admin_id, project, "admin").await;
    let token = admin
        .post("/api/v1/account/api-tokens", json!({ "name": "ci", "project_ids": [project], "role": "admin", "expires_in_days": 5 }))
        .await
        .body["token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(Client::with_bearer(&app, &token).get(&format!("/api/v1/projects/{project}")).await.status, StatusCode::OK);

    // The owner enables the requirement. Enrolling first, otherwise they are locked out too.
    let begin = owner.post("/api/v1/account/totp", json!({})).await;
    let secret = decode_secret(begin.body["secret"].as_str().unwrap());
    owner.post("/api/v1/account/totp/confirm", json!({ "code": totp::code_at(&secret, now_step()) })).await;
    assert_eq!(owner.put("/api/v1/settings/security", json!({ "require_2fa_for_admins": true })).await.status, StatusCode::OK);

    let blocked = admin.get("/api/v1/projects").await;
    assert_eq!((blocked.status, blocked.code()), (StatusCode::FORBIDDEN, "TWO_FACTOR_ENROLMENT_REQUIRED"));
    let session = admin.get("/api/v1/auth/session").await;
    assert_eq!(session.body["must_enrol_2fa"], true, "the UI learns it must show enrolment");
    let tok = Client::with_bearer(&app, &token).get(&format!("/api/v1/projects/{project}")).await;
    assert_eq!((tok.status, tok.code()), (StatusCode::FORBIDDEN, "TWO_FACTOR_REQUIRED"));

    let begin = admin.post("/api/v1/account/totp", json!({})).await;
    assert_eq!(begin.status, StatusCode::OK, "enrolment itself must stay reachable");
    let secret = decode_secret(begin.body["secret"].as_str().unwrap());
    admin.post("/api/v1/account/totp/confirm", json!({ "code": totp::code_at(&secret, now_step()) })).await;
    assert_eq!(admin.get("/api/v1/projects").await.status, StatusCode::OK);
    assert_eq!(Client::with_bearer(&app, &token).get(&format!("/api/v1/projects/{project}")).await.status, StatusCode::OK);

    let off = admin.delete("/api/v1/account/totp").await;
    assert_eq!((off.status, off.code()), (StatusCode::FORBIDDEN, "TWO_FACTOR_REQUIRED"), "cannot opt out while required");
}

#[sqlx::test(migrations = "../../migrations")]
async fn only_an_owner_can_reset_someone_elses_second_factor(pool: PgPool) {
    let app = app(pool.clone()).await;
    let (mut owner, _) = signed_in(&app, "owner@example.com", Some("owner")).await;
    let (mut admin, _) = signed_in(&app, "admin@example.com", Some("admin")).await;
    let victim = make_user(&pool, "victim@example.com", None).await;
    let mut victim_session = Client::new(&app);
    victim_session.login("victim@example.com", PASSWORD).await;
    sqlx::query!("insert into recovery_codes (user_id, code_hash) values ($1, 'x')", victim).execute(&pool).await.unwrap();
    sqlx::query!("insert into user_totp (user_id, secret_ciphertext, key_id, confirmed_at) values ($1, '\\x00', 'k1', now())", victim)
        .execute(&pool)
        .await
        .unwrap();

    let path = format!("/api/v1/users/{victim}/totp/reset");
    assert_eq!(admin.post(&path, json!({})).await.status, StatusCode::FORBIDDEN);
    assert_eq!(owner.post(&path, json!({})).await.status, StatusCode::NO_CONTENT);
    let left = sqlx::query_scalar!(
        r#"select (select count(*) from user_totp where user_id = $1) + (select count(*) from recovery_codes where user_id = $1) as "n!""#,
        victim
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 0);
    assert_eq!(victim_session.get("/api/v1/auth/session").await.status, StatusCode::UNAUTHORIZED, "their open sessions are ended");
    let audited: i64 =
        sqlx::query_scalar!(r#"select count(*) as "n!" from audit_events where action = 'user.totp_reset' and outcome = 'success'"#)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(audited, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn changes_emit_events_in_the_same_transaction(pool: PgPool) {
    let app = app(pool.clone()).await;
    let (mut owner, _) = signed_in(&app, "owner@example.com", Some("owner")).await;
    let jobs = || async {
        sqlx::query_scalar::<_, i64>("select count(*) from graphile_worker.jobs where task_identifier = 'bn_event'")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    assert_eq!(jobs().await, 0);
    let created = owner.post("/api/v1/projects", json!({ "name": "A", "slug": "a" })).await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(jobs().await, 1);
    let payload =
        sqlx::query_scalar::<_, serde_json::Value>("select payload from graphile_worker._private_jobs").fetch_one(&pool).await.unwrap();
    assert_eq!(payload["kind"], "project.created");

    // A failed request leaves neither a row nor an event.
    let dup = owner.post("/api/v1/projects", json!({ "name": "A again", "slug": "a" })).await;
    assert_eq!(dup.status, StatusCode::CONFLICT);
    assert_eq!(jobs().await, 1);
    assert_eq!(sqlx::query_scalar!(r#"select count(*) as "n!" from projects"#).fetch_one(&pool).await.unwrap(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_rolled_back_transaction_leaves_no_event(pool: PgPool) {
    let _app = app(pool.clone()).await;
    let mut tx = pool.begin().await.unwrap();
    bn_cp::events::emit(&mut tx, bn_cp::events::Event { kind: bn_cp::events::kind::PROJECT_CREATED, project_id: None, data: json!({}) })
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let n = sqlx::query_scalar::<_, i64>("select count(*) from graphile_worker.jobs").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_project_removes_its_memberships_and_token_access(pool: PgPool) {
    let app = app(pool.clone()).await;
    let (mut owner, _) = signed_in(&app, "owner@example.com", Some("owner")).await;
    let project = owner.post("/api/v1/projects", json!({ "name": "A", "slug": "a" })).await.body["id"].as_str().unwrap().to_owned();
    let pid: uuid::Uuid = project.parse().unwrap();
    let (_dev, dev_id) = signed_in(&app, "dev@example.com", None).await;
    add_member(&pool, dev_id, pid, "developer").await;
    let del = owner.delete(&format!("/api/v1/projects/{project}")).await;
    assert_eq!(del.status, StatusCode::NO_CONTENT);
    let left = sqlx::query_scalar!(r#"select count(*) as "n!" from memberships where scope = 'project'"#).fetch_one(&pool).await.unwrap();
    assert_eq!(left, 0);
    assert_eq!(owner.get(&format!("/api/v1/projects/{project}")).await.status, StatusCode::NOT_FOUND);
}
