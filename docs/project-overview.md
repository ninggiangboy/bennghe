# Project overview

## Purpose

Bennghe lets a person or a small team run a PaaS on servers they rent themselves.
One control plane manages many clusters of VPS, which may span several hosting
providers. The goal is the convenience of a hosted PaaS without handing
infrastructure or data to a third party.

Users install and operate their own copy. There is no hosted Bennghe service.

## What makes it different

Existing self-hosted panels (Coolify, Dokploy, CapRover, Dokku) cover similar
ground. Bennghe is worth building because of these properties, which the design
must preserve:

- **Agents connect out.** Each server runs an agent that dials the control plane.
  The control plane never holds SSH keys, and servers behind NAT can be managed.
- **Multi-cluster first.** Clusters are a first-class concept, not an
  afterthought bolted onto a single-server tool.
- **Lightweight agent.** The agent is a single static binary that must stay
  usable on a 1 GB VPS.
- **Keeps running without the control plane.** Apps, cron jobs and backups keep
  running while the control plane is offline.

## Scope

In scope for v1: cluster and node management; deploying apps from Git
(GitHub, GitHub Enterprise Server, GitLab, Gitea), Dockerfiles, buildpacks or
images, with watch paths for monorepos, cancellable deployments and live build
logs; preview deployments for pull requests; domains with automatic TLS or an
uploaded certificate; databases (PostgreSQL, MySQL, MongoDB, Redis) with
encrypted backups to S3-compatible storage and restore; volume backups; cron
jobs; Docker Compose stacks and a template library; project-level shared
variables; teams with role-based access, two-factor authentication, API tokens
for CI and an audit log; notifications, including CPU and memory alerts;
Cloudflare DNS, wildcard certificates, Tunnel and R2.

Forgejo is supported on a best-effort basis through the Gitea integration.

## Non-goals

- A hosted, multi-customer SaaS or billing.
- Kubernetes, autoscaling.
- DNS providers other than Cloudflare.
- High availability for the control plane, databases or the per-cluster registry.
- Relaying traffic for servers that are both behind NAT.
- A bundled metrics/logging stack (Prometheus, Loki, Grafana).

## Constraints

- **License:** AGPL-3.0, so anyone who offers a modified Bennghe as a service must
  publish their changes.
- **Single tenant per install, isolated projects inside it.** One installation
  belongs to one person or organisation. Within it, a project is a real security
  boundary: members of one project must not be able to read or affect another
  project's workloads, networks, images, data or backups.
- **Secrets stay encrypted.** Secret values are encrypted at rest and never
  stored in readable form on cluster nodes.
- **The agent is root on its node.** This is inherent to managing Docker. The
  design therefore treats the control plane's signing key as the single point
  whose compromise affects every node.

## Terminology

| Term | Meaning |
|---|---|
| Control plane | The central service users install. It holds state, serves the UI and API, and signs instructions for agents. |
| Agent | The process on every server. It connects out to the control plane and applies what it is told. |
| Cluster | A group of servers joined into one Docker Swarm over a private WireGuard network. |
| Node | One server in a cluster. Nodes can be managers or workers, and can carry the ingress or build role. |
| Ingress node | A node that receives public HTTP(S) traffic through Traefik. |
| Build node | The node in a cluster that builds images and hosts the cluster's registry. |
| Project | The unit of ownership and isolation for apps, stacks, databases and domains. |
| Desired state | The signed description of what should run, which the control plane sends to agents. |
| Epoch | An identifier of the control plane's state lineage. It changes when the control plane is restored from a backup, which tells agents not to delete anything until an admin confirms. |
