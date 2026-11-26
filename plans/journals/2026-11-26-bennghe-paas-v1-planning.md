---
title: Bennghe PaaS v1 planning
date: 2026-11-26
summary: "Brainstorm, advise and hard-mode plan for the self-hosted multi-cluster PaaS; red-team reshaped agent trust, ingress, secrets and cron design."
---

# Bennghe PaaS v1 planning

## What happened
- Greenfield repo; brainstorm then ak-advise locked scope: open-source (AGPL-3.0) self-hosted PaaS, Go control plane + outbound Go agent, React/Vite UI, Docker Swarm per cluster over WireGuard (multi-provider), per-cluster registry + build node, native GitHub/GitLab/Gitea, databases + restic backups to S3/R2, cron, compose + templates, team/RBAC/audit, Cloudflare (DNS, DNS-01 wildcard, Tunnel, R2).
- ak-plan --hard produced plans/261126-0445-self-hosted-paas-v1 (11 phases, plan.html via AK Render).
- Research corrected several assumptions: lego is v5 (ctx in Present/CleanUp); Docker 29 nftables backend has no DOCKER-USER; Cloudflare blocks gRPC on Tunnel hostnames and mTLS through the proxy; River OSS periodic jobs are not durable; pipe into restic --stdin hides dump failures (use --stdin-from-command); overlay MTU option is the underlay MTU (set 1420 → containers 1370); Traefik OSS cannot share ACME state; rootless BuildKit cannot be a Swarm service.
- Red-team: 4 reviewers, 40 findings merged into 15 groups, all accepted.

## Decision
- Two-way Ed25519 agent auth, signed desired state, cp_epoch for control-plane restore, upgrade-only channel for old agents, minisign chosen in phase 2.
- Traefik uses file provider only; agent renders routes/certs; no docker.sock in Traefik or control plane.
- Credentials live as Swarm secrets referenced by name; schedules travel in desired state to every manager's bbolt (no Swarm configs); job service names keyed on scheduled time.
- Project is a real isolation boundary: per-project overlays, admission policy, registry token auth, deploy by digest.
- Validation: keep GHES, Forgejo best-effort, 1 FTE ~36 weeks, registry GC weekly + manual, 10 templates, accept Traefik restart per new project network, product name Bennghe (prefix bn).

## Next steps
- Run /ak-cook on plans/261126-0445-self-hosted-paas-v1/plan.md starting with phase 1.
- Confirm in lab: default-network-opts for overlays; GHES manifest form URL; Forgejo OAuth clone.
- Check name availability for Bennghe before release.

> Historical work record — not durable authority. Prefer docs/specs/ADRs for current decisions.
