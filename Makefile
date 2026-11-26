# SPDX-License-Identifier: AGPL-3.0-only
# Developer entry points. CI runs the same targets.
# Tests need a Postgres with a high connection limit, for example:
#   docker run -d --name bn-dev-pg -p 127.0.0.1:54329:5432 -e POSTGRES_USER=bennghe \
#     -e POSTGRES_PASSWORD=bennghe -e POSTGRES_DB=bennghe postgres:18 -c max_connections=600
export DATABASE_URL ?= postgres://bennghe:bennghe@127.0.0.1:54329/bennghe

.PHONY: gen test lint web build db-migrate

# Regenerate committed artefacts: the OpenAPI document, the TypeScript client types and the
# offline sqlx query data.
gen: db-migrate
	BN_UPDATE_OPENAPI=1 cargo test -p bn-cp --lib committed_openapi_matches_the_code
	cargo sqlx prepare --workspace -- --all-targets
	cd web && pnpm gen

db-migrate:
	sqlx migrate run --source migrations

lint:
	cargo fmt --all --check
	SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings
	cargo deny check
	cd web && pnpm lint && pnpm typecheck

test: db-migrate
	cargo nextest run --workspace
	cd web && pnpm test

web:
	cd web && pnpm install --frozen-lockfile && pnpm build

# Release binaries embed the web UI, so it is built first.
build: web
	cargo build --release --workspace
