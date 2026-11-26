---
title: "Phase 1 foundation: control plane, web UI and the memory spike"
date: 2026-11-26
summary: "Rust kept after the spike; control plane, access-control gate, web UI and deploy files built; 65 Rust and 4 web tests pass; image build, install script and Incus lab still unverified."
---

# Phase 1 foundation: control plane, web UI and the memory spike

## What happened
- Spike: a minimal Rust agent idled at 1.2 MB RSS; the same agent in Go idled at 6.5-9.2 MB with peaks of 10.5-12.2 MB. That met the plan's "stop and ask" condition, so the user was asked and chose to keep Rust for the headroom. bollard (ReplicatedJob, update_service), graphile_worker (job enqueued in the same transaction), utoipa x-permission and wireguard-control all passed.
- Built the control plane in `crates/cp`: argon2id sign-in, sessions in a `__Host-` cookie with CSRF, TOTP with recovery codes, five-minute re-authentication, project-scoped API tokens, the `x-permission` access-control gate (`rbac::all_routes`), audit of successes and refusals, events enqueued in the caller's transaction, ACME HTTP-01 TLS with hot-swapped certificates, and the outbound URL policy.
- Built the React web UI (sign-in, setup, projects, security), the Makefile, CI workflow, Dockerfile, compose file, install script and Incus lab scripts.
- Verified against the real stack: setup, CSRF refusal and audit through `curl`; certificate issued by Pebble, served, and reused after a restart.

## Problems met
- The Postgres connection limit ran out when 19 integration tests ran in parallel; fixed by raising `max_connections` for the dev and CI database.
- `sqlx` chose `time` over `chrono` for timestamps because another crate enabled it; fixed with `sqlx.toml`.
- `openapi-typescript` does not work with TypeScript 7, so TypeScript is pinned to 5.9.
- OrbStack was stopped partway through and had to be restarted; later the Docker VM disk filled up, which stopped the image build and briefly blocked pulling any image.
- `cargo deny` flags `chrono` as unmaintained (RUSTSEC-2026-0543); it is ignored with a reason because `sqlx` and `graphile_worker` depend on it.

## Decision
Keep Rust for the control plane and the agent (user's choice after the spike; recorded in `docs/system-architecture.md`).

## Next steps
- Free Docker disk space, then build `deploy/Dockerfile` and run `install-controlplane.sh` on a clean host.
- Run `test/lab/incus-up.sh` on a Linux host with Incus; measure agent RSS on `x86_64` and on a real 512 MB VM.
- Run `make build` (release with LTO) and the CI workflow on GitHub.
- Then phase 2 (agent and transport).

> Historical work record — not durable authority. Prefer docs/specs/ADRs for current decisions.
