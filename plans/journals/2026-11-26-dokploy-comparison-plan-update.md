---
title: Dokploy comparison and Bennghe plan update
date: 2026-11-26
summary: "Compared Dokploy with the Bennghe v1 plan, added 14 selected features including PR preview deployments, and reshaped them through a 15-finding red-team and a validation pass."
---

# Dokploy comparison and Bennghe plan update

## What happened
- Ran ak-xia in compare mode against dokploy/dokploy at commit 6b02c8d5 (Apache-2.0 outside `/proprietary`; the proprietary parts — audit log viewer, SSO, SCIM, custom roles, forward auth, whitelabel, license keys — were not read). Report: `plans/reports/xia-261126-1619-dokploy-comparison.md`.
- Bennghe's core design was already stronger than Dokploy's (outbound agents vs SSH, one River queue vs three queue systems, digest deploys vs tag-based rollback registry, restic vs rclone, signed webhooks vs token URLs), so only user-facing features were worth taking.
- The user selected 2FA, re-auth, CI API tokens, custom certificate upload, cancel deployment, live build logs, build concurrency, node Docker cleanup, `${{project.KEY}}`, watch paths, stop-service volume backup, CPU/RAM alerts, template generators, and PR preview deployments. Previews became a new phase 11; E2E and release moved to phase 12.
- Red-team (4 reviewers, 40 raw findings merged to 15, all accepted) found the first draft of previews unsafe: production rollback could select a preview snapshot, the PR gate checked the author but not the pusher and built a moving ref, and preview builds shared the production BuildKit cache.

## Decision
- Previews are isolated at build and run time: `deployments.preview_id`, a separate on-demand BuildKit, their own network with a reserved subnet plus `!ClientIP` on production routers, env that never resolves project secrets, and a gate on both PR author and event sender with the verified SHA pinned.
- Preview zones are per (project, cluster); tunnel clusters need a suffix equal to the Cloudflare zone apex because Universal SSL covers only one subdomain level.
- A cluster-wide build dispatcher replaces ad-hoc gates; cancel goes through `cancelling` with compare-and-set; deployments without a build skip the slot.
- Stop-service volume backups are orchestrated by the scheduler agent through a local lease, so they work with the control plane offline; deploys wait for the lease.
- Token role is evaluated per request as the lower of token and current membership role; tokens only work on project-scoped routes.
- Validation session 2: same-registrable-domain preview zones only warn (user reversed the red-team's rejection); preview BuildKit starts on demand.
- Effort grew from about 36 to about 47 weeks for one person.

## Next steps
- Confirm in the phase 11 lab: Traefik sees the preview task's overlay IP through the Swarm VIP; minimum GitHub App permission for the collaborator permission endpoint.
- Start implementation with `/ak-cook plans/261126-0445-self-hosted-paas-v1/plan.md` from phase 1.

> Historical work record — not durable authority. Prefer docs/specs/ADRs for current decisions.
