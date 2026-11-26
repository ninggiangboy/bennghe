# Bennghe

Bennghe is a self-hosted, open-source platform for running your own fleet of VPS
clusters: deploy apps and compose stacks, run databases with backups, schedule
cron jobs, and manage domains — from one control plane you install yourself.

The name comes from Bến Nghé, the old wharf of Saigon.

## Status

Early development. The foundation is in place: a Rust control plane with sign-in, two-factor
authentication, role-based access control, audit, API tokens and its own TLS, plus a React web UI.
The agent, clusters, builds and deploys come in the following phases. The active implementation plan is
[`plans/261126-0445-self-hosted-paas-v1/plan.md`](plans/261126-0445-self-hosted-paas-v1/plan.md)
(rendered view: [`plan.html`](plans/261126-0445-self-hosted-paas-v1/plan.html)).

## Building and running

See [Development](docs/development.md) for building, testing and running from a checkout, and
`deploy/install-controlplane.sh` for installing the control plane on a server.

## Documentation

- [Development](docs/development.md) — build, test, run, and the access-control gate.
- [Project overview](docs/project-overview.md) — what Bennghe is for, who it
  serves, what it deliberately does not do, and the terms used across the project.
- [System architecture](docs/system-architecture.md) — system boundaries and the
  decisions behind them, including the alternatives that were rejected.

## License

Bennghe will be released under the GNU Affero General Public License v3.0
(AGPL-3.0).
