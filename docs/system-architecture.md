# System architecture

> This document records the boundaries and the reasons behind them. Phase 1 of
> the plan in `plans/261126-0445-self-hosted-paas-v1/` has landed the foundation
> (control plane core, access control, web shell); parts that arrive in later
> phases are marked as planned. Change this file when a decision changes rather
> than letting it drift.

## Where the code lives

| Piece | Location | State |
|---|---|---|
| Control plane (HTTP API, auth, access control, audit, events, TLS, embedded UI) | `crates/cp` (`bn-cp`) | built |
| Wire types shared with the agent | `crates/proto` (`bn-proto`), sources in `proto/` | skeleton |
| Agent | `crates/agent` (`bn-agent`) | stub; phase 2 |
| Static helpers bind-mounted into user containers | `crates/healthcheck`, `crates/envexec` | stubs; phase 5 |
| Web UI | `web/` (React, Vite), embedded into the control plane binary | built |
| Database schema | `migrations/` (forward-only, named by timestamp) | built |
| API contract | `api/openapi.yaml`, generated from the code and committed | built |
| Install and lab | `deploy/`, `test/lab/` | built; the lab needs Incus on Linux |

Inside the control plane: `rbac` (permission matrix and decision), `httpserver`
(the gate every request passes), `auth` (passwords, TOTP, sessions, API tokens),
`secrets` (encryption at rest and the instance identity), `audit`, `events`,
`jobs`, `certs`, `netpolicy`, `webui`.

## Boundaries

- **Control plane** is a Rust service with an embedded React UI and PostgreSQL. It
  is the only component users talk to. It owns all durable state, encrypts
  secrets, issues TLS certificates, and signs the desired state it sends to
  agents. It never mounts a Docker socket.
- **Agent** is a static Rust binary on every node (target: at most 10 MB RSS when idle on a 512 MB VPS). It dials the control plane, verifies
  that instructions are signed by the control plane key it pinned at enrolment,
  and applies them. Node-level state (WireGuard, firewall, files on the node) is
  applied by each node's own agent. Cluster-level state (Swarm services,
  networks, secrets) is applied only by the agent on the current Swarm leader.
- **Cluster** is a Docker Swarm whose nodes talk to each other only over a
  WireGuard mesh. Each project gets its own overlay network; each database gets a
  private network shared only with the apps linked to it.
- **Ingress** is Traefik on nodes labelled for ingress. It reads only
  configuration files the agent writes, never the Docker API.
- **Per-cluster registry and builder** live on the build node and are reachable
  only over WireGuard.

## Decisions

| Decision | Rejected alternatives | Why |
|---|---|---|
| Rust for both control plane and agent | Go for both; Go control plane with a Rust agent | The agent must stay under 10 MB RSS on a 512 MB VPS. The phase 1 spike measured a minimal Rust agent at 1.2 MB and the same agent in Go at 6.5 to 12 MB, peaking above the budget. The owner kept Rust for the headroom the full agent will need (`plans/reports/spike-agent-memory-261126.md`). Costs: the OpenAPI spec is generated from code instead of written first, the compose parser and the Cloudflare DNS-01 step are written by us (no Rust equivalents of compose-go or lego's DNS providers), builds call `buildctl` in a container, and reviewing and debugging Rust falls on the maintainer. |
| Agents connect out to the control plane | Control plane SSHes into servers (Coolify, Kamal) | No SSH keys to steal from the control plane, and servers behind NAT can be managed. Cost: the agent and its self-update must be maintained. |
| Docker Swarm per cluster | Plain Docker with one service per node; a custom multi-node scheduler; Kubernetes | Swarm gives replicas, rescheduling and rolling updates without building a scheduler. Cost: a smaller ecosystem, and databases are not highly available. Revisit if Swarm support ends. |
| WireGuard mesh under Swarm, always | Rely on a provider's private network | Clusters can span providers, and Swarm's ports are never exposed to the internet. Cost: a fixed MTU budget (overlay networks must carry an explicit MTU) and nodes need a reachable UDP port. |
| WebSocket with mutual Ed25519 authentication and signed desired state | gRPC; mutual TLS | gRPC does not pass Cloudflare Tunnel hostnames, and client certificates do not pass Cloudflare's proxy. Application-level signatures work behind any TLS-terminating proxy and also protect agents from an impostor control plane. |
| Control plane issues all certificates and pushes them to Traefik | Let Traefik run ACME itself | Traefik's open-source edition cannot share ACME state between instances, so several ingress nodes would request duplicate certificates and hit rate limits. |
| Traefik uses only the file provider, written by the agent | Traefik's Swarm provider | The Swarm provider needs a manager's Docker socket inside the internet-facing process, and polling can route to stopped tasks. Routing to Swarm service VIPs avoids both. |
| Credentials reach the cluster only as Swarm secrets referenced by name | Credentials in schedules, Swarm configs or environment variables | Swarm configs and service specs are readable by anyone with manager API access; secrets are not. Backups can still run while the control plane is offline. |
| Schedules travel inside desired state to every manager's local store | Store schedules as Swarm configs | One synchronisation mechanism instead of two, and no size limit or cleanup of config revisions. |
| Projects are isolated: separate networks, admission policy for workloads, per-project registry pull rights, deploys by image digest | Projects as a UI grouping only | The owner chose real isolation. Cost: Traefik must join each project network, which restarts it briefly when a project gets its first domain in a cluster. |
| Registry and builder per cluster | Require an external registry; accept only prebuilt images | "Push and it runs" works out of the box. Cost: one more component per cluster, and the registry is a single point for new deploys. |
| Preview deployments are isolated from production at build and run time: a separate BuildKit instance and cache, their own network, deployment history, secrets and environment, and builds only when both the pull request author and the pusher have write access | Previews that inherit the app's environment and share its build cache (as Dokploy does) | Pull request code is unreviewed. Sharing secrets, cache or rollback history would let it reach production. Cost: a second BuildKit instance on the build node (started on demand) and separate preview configuration. |
| The control plane serves its own TLS | A Caddy container in front | One certificate system instead of two. |
| Every API operation declares its permission in the OpenAPI document (`x-permission`), and the access-control table is built from that document | A separate route-to-permission table; permission checks inside handlers | A route without a declared permission cannot be served, and CI fails when one is added. Cost: the document is generated from code (utoipa) rather than written first. |
| Sensitive operations need a password or code check within five minutes, and API tokens can never satisfy it | Re-checking only at sign-in; letting tokens do everything their user can | A stolen session or leaked CI token cannot mint tokens, reset two-factor authentication or delete a project. Cost: those operations exist only in the web UI. |
| API tokens are project-scoped and their effective role is recomputed on every request as the lower of the token's role and its owner's current role | Tokens that keep the role they were created with | Demoting or removing a user takes effect on their tokens immediately. |
| Sessions are opaque random tokens stored only as SHA-256, in a `__Host-` prefixed cookie, with a CSRF token tied to the session; the token is replaced when the second factor succeeds | Signed JWTs; long-lived cookies without a CSRF token | Sessions can be revoked at once and a captured pre-2FA token is worthless. Cost: one database lookup per request. |
| AGPL-3.0 | Apache-2.0, MIT | Keeps hosted forks open. Cost: some companies avoid AGPL software. |

## Known limits by design

- The control plane's own certificate and key are stored as files (mode 0600) in the data
  directory, next to `master.key`; certificates for apps (phase 4) are stored encrypted in the
  database.
- Certificates are renewed when 30 days remain, on a fixed schedule rather than from ACME
  renewal information.

- A cron run that falls exactly on a Swarm leader change may be skipped; missed
  runs are not replayed.
- Pruning old backups needs the control plane online.
- Restoring the control plane from a backup puts agents in a hold mode until an
  admin confirms, so nothing is deleted based on stale state.
