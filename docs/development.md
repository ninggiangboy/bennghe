# Development

How to build, test and run Bennghe from a checkout.

## What you need

- Rust (stable; `rust-toolchain.toml` pins the channel and components)
- Node 26 and pnpm 12 for the web UI
- Docker, for a Postgres to test against
- `sqlx-cli`, `cargo-nextest` and `cargo-deny` (`brew install sqlx-cli cargo-nextest cargo-deny`,
  or `cargo install`)

## Database for development and tests

Every integration test creates its own database, so Postgres needs a high connection limit.
Use a fixed port so a stale container is easy to spot:

```sh
docker run -d --name bn-dev-pg -p 127.0.0.1:54329:5432 \
  -e POSTGRES_USER=bennghe -e POSTGRES_PASSWORD=bennghe -e POSTGRES_DB=bennghe \
  postgres:18 -c max_connections=600
```

The Makefile defaults `DATABASE_URL` to `postgres://bennghe:bennghe@127.0.0.1:54329/bennghe`.
Stop the container when you are done (`docker rm -f bn-dev-pg`).

## Everyday commands

| Command | What it does |
|---|---|
| `make test` | Applies migrations, then runs every Rust test (`cargo nextest`) and the web tests |
| `make lint` | `rustfmt`, `clippy -D warnings`, `cargo deny`, and the web lint and typecheck |
| `make gen` | Regenerates `api/openapi.yaml`, the web API types and the offline `sqlx` data |
| `make web` | Builds the web UI into `web/dist`, which the control plane embeds |
| `make build` | Release build of every binary, after the web UI |

The SQL queries are checked against the real schema at compile time. CI and anyone without a
database build with `SQLX_OFFLINE=true`, which reads the committed `.sqlx/` directory. After
changing a query or a migration, run `make gen` and commit the result.

## The access-control gate

Every API operation carries `x-permission` (`action`, `scope`, and `reauth` when it needs a recent
password check). `crates/cp/src/rbac/policy.rs` holds the matrix of actions, scopes and minimum
roles. The test `cargo nextest run -p bn-cp rbac::all_routes` fails when:

- a route has no `x-permission`, or names an action missing from the matrix;
- the declared scope differs from the matrix, or a `project`-scoped route lacks `{project_id}`;
- a matrix row has no route, or two routes share an action;
- the `reauth` flag and `REAUTH_ACTIONS` disagree;
- `api/openapi.yaml` no longer matches what the code generates.

When you add a route: write the handler with `#[utoipa::path(... extensions(("x-permission" = json!({...}))))]`,
register it in `crates/cp/src/api/mod.rs`, add its row to the matrix, and run `make gen`.
A project-scoped route must name its path parameter `project_id`; sensitive operations
(see `REAUTH_ACTIONS`) must set `"reauth": true`.

## Running the control plane locally

```sh
make web                                   # once, so the UI is embedded
export BN_DATABASE_URL=postgres://bennghe:bennghe@127.0.0.1:54329/bennghe
export BN_DATA_DIR=$(mktemp -d) BN_TLS_MODE=off BN_HTTP_ADDR=127.0.0.1:8080 BN_INSECURE_COOKIES=true
cargo run -p bn-cp -- init                 # creates master.key and signing.key in BN_DATA_DIR
cargo run -p bn-cp -- setup-token          # prints the one-time token for /setup
cargo run -p bn-cp -- serve
```

`BN_INSECURE_COOKIES` drops the `__Host-` cookie prefix and the `Secure` flag so cookies work over
plain HTTP; never set it in production. For UI work run `pnpm dev` in `web/`; it proxies `/api` to
`http://127.0.0.1:8080`.

A `master.key` belongs to the database it first touched. The control plane refuses to start with a
different key, so use a fresh database whenever you create a new data directory.

### Configuration

Settings come from environment variables, optionally from a TOML file named by `BN_CONFIG`;
environment wins. `BN_DATABASE_URL` is required. The others: `BN_DATA_DIR` (default
`/var/lib/bennghe`), `BN_TLS_MODE` (`acme` or `off`), `BN_PUBLIC_DOMAIN` (required for `acme`),
`BN_ACME_EMAIL`, `BN_ACME_DIRECTORY`, `BN_ACME_ROOT_CA` (for a private or test ACME server),
`BN_HTTP_ADDR`, `BN_HTTPS_ADDR`, `BN_TRUSTED_PROXIES` (CIDR list whose `X-Forwarded-For` is
believed), `BN_INSECURE_COOKIES`.

### Recovery commands

Run on the host that has the key files:

- `controlplane admin reset-2fa --user <email>` removes two-factor authentication from an account
  whose owner lost every device and recovery code. It is audited.
- `controlplane setup-token` prints a new one-time setup token while no owner exists.

## Testing TLS against a local ACME server

Pebble, the Let's Encrypt test server, accepts any name and validates nothing, which is enough to
exercise issuance, serving and reuse after a restart:

```sh
docker run -d --name bn-pebble -p 127.0.0.1:14000:14000 -p 127.0.0.1:15000:15000 \
  -e PEBBLE_VA_ALWAYS_VALID=1 ghcr.io/letsencrypt/pebble:latest
docker cp bn-pebble:/test/certs/pebble.minica.pem /tmp/pebble-minica.pem
BN_TLS_MODE=acme BN_PUBLIC_DOMAIN=cp.test BN_ACME_DIRECTORY=https://localhost:14000/dir \
  BN_ACME_ROOT_CA=/tmp/pebble-minica.pem BN_HTTP_ADDR=127.0.0.1:8080 BN_HTTPS_ADDR=127.0.0.1:8443 \
  cargo run -p bn-cp -- serve
```

## Installing on a server

`deploy/install-controlplane.sh --domain <name> --email <address>` builds the image, creates the
keys, prints the setup token and waits for HTTPS. See the script header. **Back up `master.key`
immediately**; without it every stored secret is unrecoverable.

## The lab

`test/lab/incus-up.sh` creates Ubuntu virtual machines with Docker and WireGuard on a Linux host
with Incus, for the phases that need real kernels. `test/lab/incus-down.sh` removes them.
`test/spike/` holds the throwaway agent memory measurement from phase 1; its result is in
`plans/reports/spike-agent-memory-261126.md`.

## Notes

- TypeScript is pinned to 5.9: `openapi-typescript` uses the TypeScript JavaScript API, which
  TypeScript 7 does not provide. Move to 7 when the generator supports it.
- `cargo deny` ignores RUSTSEC-2026-0543 (`chrono` is unmaintained). `sqlx` and `graphile_worker`
  depend on it; see `deny.toml`.
