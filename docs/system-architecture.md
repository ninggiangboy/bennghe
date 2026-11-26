# System architecture

> No code exists yet, so this document records **design intent**. The
> implementation plan in `plans/261126-0445-self-hosted-paas-v1/` owns execution
> detail. When code lands, link boundaries below to their packages, and change
> this file when a decision changes rather than letting it drift.

## Boundaries

- **Control plane** is a Go service with an embedded React UI and PostgreSQL. It
  is the only component users talk to. It owns all durable state, encrypts
  secrets, issues TLS certificates, and signs the desired state it sends to
  agents. It never mounts a Docker socket.
- **Agent** is a Go binary on every node. It dials the control plane, verifies
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
| The control plane serves its own TLS | A Caddy container in front | One certificate system instead of two. |
| AGPL-3.0 | Apache-2.0, MIT | Keeps hosted forks open. Cost: some companies avoid AGPL software. |

## Known limits by design

- A cron run that falls exactly on a Swarm leader change may be skipped; missed
  runs are not replayed.
- Pruning old backups needs the control plane online.
- Restoring the control plane from a backup puts agents in a hold mode until an
  admin confirms, so nothing is deleted based on stale state.
