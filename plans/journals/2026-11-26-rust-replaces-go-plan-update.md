---
title: Bennghe plan moves from Go to Rust
date: 2026-11-26
summary: "After an advisory interview the user chose all-Rust to meet a 10 MB agent RSS target on 512 MB VPS; the v1 plan and architecture doc were rewritten with a memory spike as a blocking gate."
---

# Bennghe plan moves from Go to Rust

## What happened
- The user asked whether to use Rust instead of Go. The only driver they gave was an agent that must idle under 10 MB RSS on a 512 MB VPS. They are equally fluent in both languages and AI writes the code, so learning cost was not a factor. They chose Rust for both control plane and agent, against the advice to keep Go on the control plane.
- Checked crates.io and docs for replacements. Most Go libraries have a Rust counterpart (`bollard`, `sqlx`, `instant-acme`, `wireguard-control`, `redb`). Real gaps: no compose-go equivalent, no DNS-01 providers like lego's, no reliable BuildKit client, and code-first OpenAPI instead of spec-first.
- The advice named `tonic`, which was wrong: the plan uses WebSocket plus protobuf because gRPC does not pass through Cloudflare Tunnel. The plan now says so explicitly.
- Updated `plan.md`, all 12 phase files, `plan.page.yaml` and `plan.html`, and `docs/system-architecture.md`. Effort rose from about 47 to about 53 weeks (an unverified estimate).

## Decision
- The language change takes effect only after a spike in phase 1 measures a Rust agent at 10 MB RSS or less on a 512 MB VM. If it does not, or a Go agent measured the same way is already at 10 MB or less, work stops and the user decides.
- Compose support becomes a documented subset checked against `docker compose config`; unsupported features are rejected by name.

## Not verified
- Any RSS number, `bollard` behaviour with Docker 29, transactional enqueue with `graphile_worker`, `x-permission` through `utoipa`, and the 4.7-week effort increase.
- The updated plan has not been through red-team or validate again.
