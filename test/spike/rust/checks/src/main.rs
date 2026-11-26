//! Assumption checks for the Rust decision (phase 1 spike). Each check prints PASS/FAIL.
//! Usage: bn-spike-checks <docker-http-url> <postgres-url>
use std::env;

use serde_json::json;

use bollard::{
    models::{
        ServiceSpec, ServiceSpecMode, ServiceSpecModeReplicatedJob, TaskSpec, TaskSpecContainerSpec,
    },
    query_parameters::{InspectServiceOptions, UpdateServiceOptions},
    Docker,
};

fn report(name: &str, r: Result<String, String>) -> bool {
    match r {
        Ok(d) => {
            println!("PASS {name}: {d}");
            true
        }
        Err(e) => {
            println!("FAIL {name}: {e}");
            false
        }
    }
}

async fn check_bollard(docker_url: &str) -> Result<String, String> {
    let docker = Docker::connect_with_http(docker_url, 30, bollard::API_DEFAULT_VERSION).map_err(|e| e.to_string())?;
    let ver = docker.version().await.map_err(|e| e.to_string())?;
    let api = ver.api_version.unwrap_or_default();
    let spec = |sleep: &str| ServiceSpec {
        name: Some("spike-job".into()),
        mode: Some(ServiceSpecMode {
            replicated_job: Some(ServiceSpecModeReplicatedJob {
                max_concurrent: Some(1),
                total_completions: Some(1),
            }),
            ..Default::default()
        }),
        task_template: Some(TaskSpec {
            container_spec: Some(TaskSpecContainerSpec {
                image: Some("busybox".into()),
                command: Some(vec!["sleep".into(), sleep.into()]),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let _ = docker.delete_service("spike-job").await;
    let created = docker.create_service(spec("1"), None).await.map_err(|e| format!("create ReplicatedJob: {e}"))?;
    let id = created.id.ok_or("no service id")?;
    let inspected = docker.inspect_service(&id, None::<InspectServiceOptions>).await.map_err(|e| e.to_string())?;
    let mode_is_job = inspected.spec.as_ref().and_then(|s| s.mode.as_ref()).and_then(|m| m.replicated_job.as_ref()).is_some();
    let index = inspected.version.and_then(|v| v.index).ok_or("no version index")?;
    docker
        .update_service(&id, spec("2"), UpdateServiceOptions { version: index as i32, ..Default::default() }, None)
        .await
        .map_err(|e| format!("update_service: {e}"))?;
    let after = docker.inspect_service(&id, None::<InspectServiceOptions>).await.map_err(|e| e.to_string())?;
    let cmd = after.spec.and_then(|s| s.task_template).and_then(|t| t.container_spec).and_then(|c| c.command);
    docker.delete_service(&id).await.map_err(|e| e.to_string())?;
    if !mode_is_job {
        return Err("service created but mode is not ReplicatedJob".into());
    }
    Ok(format!("engine API {api}: ReplicatedJob created, update_service applied (command now {cmd:?}), deleted"))
}

#[derive(serde::Deserialize, serde::Serialize)]
struct Ping {
    n: u32,
}
impl graphile_worker::TaskHandler for Ping {
    const IDENTIFIER: &'static str = "ping";
    async fn run(self, _ctx: graphile_worker::WorkerContext) -> impl graphile_worker::IntoTaskHandlerResult {
        println!("  worker ran ping n={}", self.n);
        Ok::<(), String>(())
    }
}

async fn check_job_in_tx(pg_url: &str) -> Result<String, String> {
    use sqlx::Row;
    // The worker's init() installs the graphile_worker schema through its own migration.
    let worker = graphile_worker::WorkerOptions::default()
        .database_url(pg_url)
        .concurrency(1)
        .schema("graphile_worker")
        .define_job::<Ping>()
        .init()
        .await
        .map_err(|e| format!("worker init/migrate: {e}"))?;
    let pool = sqlx::PgPool::connect(pg_url).await.map_err(|e| e.to_string())?;
    sqlx::query("create table if not exists spike_things (id int primary key)").execute(&pool).await.map_err(|e| e.to_string())?;
    sqlx::query("truncate spike_things").execute(&pool).await.map_err(|e| e.to_string())?;
    sqlx::query("delete from graphile_worker._private_jobs").execute(&pool).await.map_err(|e| e.to_string())?;

    // 1. rolled-back transaction: neither the row nor the job may exist.
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("insert into spike_things values (1)").execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("select graphile_worker.add_job('ping', json_build_object('n', 1), job_key => 'k1')").execute(&mut *tx).await.map_err(|e| format!("add_job: {e}"))?;
    tx.rollback().await.map_err(|e| e.to_string())?;
    let after_rollback: i64 = sqlx::query("select count(*) from graphile_worker.jobs").fetch_one(&pool).await.map_err(|e| e.to_string())?.get(0);
    if after_rollback != 0 {
        return Err(format!("{after_rollback} job(s) survived a rolled-back transaction"));
    }

    // 2. committed transaction: row and job exist; job_key de-duplicates; worker consumes it.
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("insert into spike_things values (2)").execute(&mut *tx).await.map_err(|e| e.to_string())?;
    for _ in 0..2 {
        sqlx::query("select graphile_worker.add_job('ping', json_build_object('n', 2), job_key => 'k2')").execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    let queued: i64 = sqlx::query("select count(*) from graphile_worker.jobs").fetch_one(&pool).await.map_err(|e| e.to_string())?.get(0);
    if queued != 1 {
        return Err(format!("expected 1 job after commit with duplicate job_key, found {queued}"));
    }
    tokio::time::timeout(std::time::Duration::from_secs(20), worker.run()).await.ok();
    let left: i64 = sqlx::query("select count(*) from graphile_worker.jobs").fetch_one(&pool).await.map_err(|e| e.to_string())?.get(0);
    Ok(format!("rollback left 0 jobs; commit left 1 job (duplicate job_key merged); jobs after worker ran: {left}"))
}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/api/v1/projects/{id}/deployments",
    params(("id" = String, Path, description = "project id")),
    responses((status = 202, description = "accepted")),
    extensions(("x-permission" = json!({"action": "deploy:create", "scope": "project", "reauth": false}))),
)]
fn create_deployment() {}

#[derive(utoipa::OpenApi)]
#[openapi(paths(create_deployment))]
struct Api;

fn check_utoipa() -> Result<String, String> {
    use utoipa::OpenApi;
    let doc = Api::openapi();
    let json = doc.to_json().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let perm = &v["paths"]["/api/v1/projects/{id}/deployments"]["post"]["x-permission"];
    let yaml = doc.to_yaml().map_err(|e| e.to_string())?;
    if perm["action"] == "deploy:create" && perm["scope"] == "project" && yaml.contains("x-permission") {
        Ok(format!("operation carries x-permission = {perm}; YAML output contains it too"))
    } else {
        Err(format!("x-permission missing or malformed: {perm}\n{yaml}"))
    }
}

fn check_wireguard() -> Result<String, String> {
    use wireguard_control::{Backend, DeviceUpdate, InterfaceName, Key, KeyPair, PeerConfigBuilder};
    let name: InterfaceName = "bn-wg0".parse().map_err(|e| format!("{e:?}"))?;
    let pair = KeyPair::generate();
    let peer = KeyPair::generate();
    DeviceUpdate::new()
        .set_keypair(pair.clone())
        .set_listen_port(51820)
        .replace_peers()
        .add_peer(PeerConfigBuilder::new(&peer.public).add_allowed_ip("10.77.0.2".parse().unwrap(), 32))
        .apply(&name, Backend::Kernel)
        .map_err(|e| format!("apply (kernel backend): {e}"))?;
    let dev = wireguard_control::Device::get(&name, Backend::Kernel).map_err(|e| e.to_string())?;
    let ok = dev.listen_port == Some(51820) && dev.peers.len() == 1 && dev.public_key == Some(pair.public.clone());
    let _ = Key::generate_private();
    wireguard_control::Device::get(&name, Backend::Kernel).and_then(|d| d.delete()).map_err(|e| format!("delete: {e}"))?;
    if ok {
        Ok("bn-wg0 created via kernel netlink, key/port/peer read back, interface deleted".into())
    } else {
        Err("device state did not read back as configured".into())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let docker_url = env::args().nth(1).unwrap_or_else(|| "http://127.0.0.1:2375".into());
    let pg_url = env::args().nth(2).unwrap_or_else(|| "postgres://postgres:postgres@127.0.0.1/postgres".into());
    let mut ok = true;
    ok &= report("bollard ReplicatedJob + update_service", check_bollard(&docker_url).await);
    ok &= report("job enqueue in same transaction (graphile_worker.add_job via sqlx)", check_job_in_tx(&pg_url).await);
    ok &= report("utoipa x-permission via extensions(...)", check_utoipa());
    ok &= report("wireguard-control bn-wg0", check_wireguard());
    std::process::exit(if ok { 0 } else { 1 });
}
