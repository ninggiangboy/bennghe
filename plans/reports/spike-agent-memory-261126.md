# Spike: agent memory and the four Rust assumptions

Date: 2026-11-26. Plan: [self-hosted-paas-v1, phase 1, task 0](../261126-0445-self-hosted-paas-v1/phase-01-foundation.md).

## Result

**The spike gate stops here and needs your decision.** The Rust agent is far under the budget (1.2 MB idle). The Go agent measured under the same conditions is also at or under 10 MB at the five-minute mark (8.0 and 9.2 MB), which is the plan's explicit stop condition: "the Go agent already ≤ 10 MB, so the reason for changing language no longer holds; stop and ask the user". All four other assumptions passed. No code beyond the spike has been written.

## Idle RSS after five minutes

Both agents do the same work: one WSS connection (TLS, protobuf frames) to a fake control plane, plus Docker `info` and `list services` every 15 seconds. The server confirmed a frame about every 15 seconds for each run, so the agents were really polling.

| Agent | Binary on disk | RSS at 300 s | RSS at 330 s | Peak (`VmHWM`) | Range from 20 s to 330 s |
|---|---|---|---|---|---|
| Rust, `musl`, `current_thread`, `opt-level="z"`, LTO fat, `panic="abort"`, stripped, default allocator | 1.9 MB | **1.18 MB** | **1.18 MB** | 1.9 MB | 1.2–1.9 MB |
| Go, default settings | 9.2 MB | 9.1 MB | 9.2 MB | 11.1 MB | 6.5–11.1 MB |
| Go, `GOMEMLIMIT=8MiB`, `GOGC=50` | 9.2 MB | 7.8 MB | 8.0 MB | 12.2 MB | 7.2–12.2 MB |

The Rust build needed none of the fallback variants (`mimalloc`, other opt levels), because it passed with the first configuration. The Go agent oscillates as its garbage collector runs: the 5-minute sample sits under 10 MB, but its peaks do not.

How much each agent has left under the 10 MB budget is the real difference. Rust has about 8.8 MB of room; Go has between 0.8 MB and 3.5 MB at the sampled points and exceeds the budget at its peaks. The real agent adds Ed25519 verification, `redb`, `chrono-tz`, log streaming and WireGuard handling, and none of that is in the spike.

## The four assumptions

| Assumption | Result | Evidence |
|---|---|---|
| `bollard` creates a `ReplicatedJob` and calls `update_service` on Docker API ≥ 1.44 | **Pass** | Engine 29.10.1, API 1.57. Service created in `ReplicatedJob` mode, `update_service` with the version index changed its command, service deleted. |
| Enqueue a job in the same transaction as the operation with `graphile_worker` | **Pass** | `graphile_worker.add_job(...)` called from a separate `sqlx` 0.9 transaction: rollback left 0 jobs, commit left 1 job (a repeated `job_key` merged into one), and the worker ran it. |
| `utoipa` exports `x-permission` through `extensions(...)` | **Pass** | The operation's JSON and YAML both carry `{"action":"deploy:create","reauth":false,"scope":"project"}`. The macro wants a bare `json!(...)`, so each file with routes needs `use serde_json::json`. |
| `wireguard-control` creates and configures `bn-wg0` | **Pass** | Kernel backend via netlink: key, listen port 51820 and one peer read back correctly; interface deleted. |

The versions that built are `tokio` 1.54, `tokio-tungstenite` 0.30, `rustls` 0.23 with `ring`, `prost` 0.14, `bollard` 0.21, `sqlx` 0.9, `graphile_worker` 0.15, `utoipa` 6 and `wireguard-control` 2. `rustls` 0.24 is only a dev release, so the plan should say 0.23.

## What this spike did not measure

- **Setup differs from the plan.** The plan asks for an Incus VM with 512 MB. This machine has no Incus, so I used a privileged `docker:dind` container with a 512 MB cgroup limit on OrbStack (Linux 7.2, aarch64), a single-node Swarm and one service. RSS of an idle process does not depend on the VM size, but kernel and allocator behaviour could differ on a real VPS.
- **`x86_64` was not measured**, only `aarch64`. Emulation under OrbStack would give a meaningless figure. The success criterion needs both.
- The agent is minimal and the cluster has one service. RSS will grow with the real feature set and with larger service lists.
- The checks ran inside the same container as the engine; `wireguard-control` worked there because the OrbStack kernel includes WireGuard.

## Decision for you

1. **Continue with Rust.** The reason is now margin, not the threshold: Rust leaves room for the full agent, while Go would sit close to 10 MB and above it at peaks as soon as it grows. The plan already estimates the cost at about +4.7 weeks (unverified), mostly the compose parser, the DNS-01 Cloudflare step and `buildctl` plumbing. The four assumptions that could have blocked Rust all passed.
2. **Return to Go.** The idle number passes, the plan was written for Go before the Rust revision (commit `66ddc0b` and earlier), and about 4.7 weeks of estimated effort come back. The risk is the thin margin and the garbage-collector peaks, which would have to be managed for the whole life of the agent.
3. **Measure first.** Add the realistic extras to both spike agents (verification, `redb`, a service list of a few hundred entries, a log stream) and measure on `x86_64` before deciding. This costs a day or two.

I lean towards option 1 or 3, but this is your call: you made the language choice on the 10 MB argument, and that argument is now partly weakened.

## Reproduce

Sources are in `test/spike/` (`rust/agent`, `rust/server`, `rust/checks`, `go`, `measure.sh`). Build in `rust:alpine` and `golang:alpine` containers; run in `docker:dind` with `--privileged --memory 512m` after `docker swarm init` and a `busybox sleep` service; run `measure.sh <label> <binary>`. RSS comes from `/proc/<pid>/status` (`VmRSS`), because the BusyBox `ps` has no `-o rss`. The throwaway containers were removed afterwards; the Docker volumes `bn-spike-cargo` and `bn-spike-gomod` hold only build caches.

## Unresolved questions

- Which of the three options do you want?
- Do you want the Go spike code deleted now (the plan says to delete it after this report), or kept until you decide?
