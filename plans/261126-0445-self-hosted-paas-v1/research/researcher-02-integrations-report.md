# Research 02: External Integrations Fact-Check

Date: 2026-11-26. Scope: verify integration assumptions for the Go control plane, Go agent, React UI, and Postgres PaaS. Evidence comes from official docs, upstream source code, and the GitHub/Codeberg release APIs on 2026-11-26.

## Bottom line

Most assumptions hold. Five findings change the plan:

1. **lego is now v5** (`github.com/go-acme/lego/v5`, v5.0.0 released 2026-05-11, latest v5.7.0). `challenge.Provider` now takes a `context.Context`, so v4 examples will not compile.
2. **Docker 29 has an experimental nftables backend with no `DOCKER-USER` chain.** The UFW-bypass mitigation therefore depends on which firewall backend the node uses.
3. **gRPC does not work through a Cloudflare Tunnel public hostname**, and Cloudflare Access does not cover proxied gRPC. Agent transport should be WebSocket, not gRPC, if it may sit behind Cloudflare.
4. **River's open-source periodic jobs are not durable.** Schedules live in memory, so runs can be skipped when the leader changes.
5. **Pipe-based restic backup (`pg_dump | restic --stdin`) hides pg_dump failures.** Use `--stdin-from-command` instead.

## 1. GitHub App Manifest flow: TRUE (GHES PARTLY)

- **Evidence:** POST a `manifest` form to `https://github.com/settings/apps/new` (or `/organizations/ORG/settings/apps/new`) with an optional `state`. GitHub redirects to `redirect_url?code=…&state=…`. Then call `POST /app-manifests/{code}/conversions`, which needs no auth header. It returns `id`, `slug`, `client_id`, `client_secret`, `webhook_secret` and `pem`. The whole flow must finish within 1 hour.
- **GHES:** The GHES 3.22 REST docs give `http(s)://HOSTNAME/api/v3/app-manifests/CODE/conversions`. The GHES guide still shows `github.com/settings/apps/new` URLs, so the form URL `https://HOSTNAME/settings/apps/new` is inferred, not documented. Manifests do not support enterprise-owned apps or enterprise permissions.
- **Clone:** Use `https://x-access-token:TOKEN@host/owner/repo.git`. This needs `contents: read`. The token expires after 1 hour and can be narrowed with `repositories` and `permissions`.
- **Statuses and checks:** `POST /repos/{o}/{r}/statuses/{sha}` needs Commit statuses write. `POST /repos/{o}/{r}/check-runs` needs Checks write, and only GitHub Apps can create check runs. There is a limit of 1000 statuses per SHA+context.
- **Gotchas:**
  - Mint a fresh installation token per build, because a build can outlast the 1-hour expiry.
  - Never log tokenized clone URLs. Pass the token through a `GIT_ASKPASS` script or an `http.extraHeader` setting instead.
  - Store `pem` and `client_secret` encrypted.
  - Bind `state` to the user's session.
- **Sources:**
  - https://docs.github.com/en/apps/sharing-github-apps/registering-a-github-app-from-a-manifest
  - https://docs.github.com/en/enterprise-server@3.22/rest/apps/apps#create-a-github-app-from-a-manifest
  - https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/authenticating-as-a-github-app-installation
  - https://docs.github.com/en/rest/authentication/permissions-required-for-github-apps
  - https://docs.github.com/en/rest/checks/runs

## 2. GitLab: TRUE

- **OAuth2:** Apps can be user-, group- or instance-owned, and instance-owned apps can be marked trusted. Self-managed instances support this on all tiers. Authorization code with PKCE is supported. Scopes include `api`, `read_api`, `read_repository`, `write_repository` and `read_user`.
- **Tokens:** Access tokens expire after 7200 s, and each refresh rotates both the access and refresh tokens.
- **Webhooks:** `POST /projects/:id/hooks` takes `token`, which arrives as the `X-Gitlab-Token` header (plain shared secret, not HMAC). GitLab 19.0+ adds `signing_token` (`whsec_<base64 32B>`), which produces an HMAC `webhook-signature` header. Creating a hook requires the Maintainer role or higher. Changing the hook URL resets the token.
- **Status:** `POST /projects/:id/statuses/:sha` accepts `state` ∈ {pending, running, success, failed, canceled}, plus `name`/`context`, `target_url` (≤255 chars), `description` and `ref`.
- **Clone:** Use `https://oauth2:<token>@host/group/proj.git`. This needs the `read_repository` scope.
- **Gotchas:**
  - Refresh rotation invalidates the old refresh token, so serialize refreshes per connection with a row lock or River unique job to avoid races.
  - The 2-hour expiry means long builds should refresh the token just before cloning.
  - Verify `X-Gitlab-Token` with a constant-time compare.
  - Only use `signing_token` after detecting GitLab 19.0 or later.
- **Sources:**
  - https://docs.gitlab.com/integration/oauth_provider/
  - https://docs.gitlab.com/api/oauth2/
  - https://docs.gitlab.com/api/project_webhooks/
  - https://docs.gitlab.com/api/commits/

## 3. Gitea / Forgejo: TRUE (still compatible for this surface)

- **OAuth2:** Gitea is an OAuth2/OIDC provider (`/login/oauth/authorize`, `/login/oauth/access_token`) that supports PKCE. Granular scopes such as `read:repository` exist since 1.23.
- **OAuth tokens for git:** Gitea's `services/auth/basic.go` accepts an OAuth2 access token as the Basic-auth password or username, so git-over-HTTPS clone works. Forgejo inherits the same code.
- **Webhooks:** Gitea sends `X-Gitea-Signature`, which is a hex HMAC-SHA256 of the raw body with no prefix. It also sends `X-Hub-Signature-256: sha256=…`. Forgejo's `services/webhook/deliver.go` sends both `X-Forgejo-Signature` and `X-Gitea-Signature` with the same value, plus the `X-Hub-*` and `X-Gogs-*` variants.
- **Commit status:** `POST /repos/{owner}/{repo}/statuses/{sha}` is the same on both.
- **Divergence:** Forgejo became a hard fork in Feb 2024 and states it "will strive to remain compatible with the Gitea API". It still cherry-picks Gitea changes weekly. Current releases are Forgejo v17.0.2 (2026-11-18; v15 is the LTS) and Gitea v28.4.0 (GitHub release tag observed). Divergence so far is in Actions and federation features, not in OAuth, hooks or statuses.
- **Gotchas:**
  - Use one adapter with a per-instance base URL, and verify `X-Gitea-Signature` (present on both) over the raw body.
  - Hooks without a secret send empty signatures, so reject them.
  - There is no timestamp header, so deduplicate replays by the `X-Gitea-Delivery` UUID.
  - Run integration tests against both Gitea and Forgejo containers to catch future drift.
- **Sources:**
  - https://docs.gitea.com/development/oauth2-provider
  - https://docs.gitea.com/usage/webhooks
  - https://forgejo.org/2024-02-forking-forward/
  - https://forgejo.org/docs/v17.0/user/repository/webhooks/
  - https://codeberg.org/forgejo/forgejo/src/branch/forgejo/services/webhook/deliver.go
  - https://github.com/go-gitea/gitea/blob/main/services/auth/basic.go

## 4. Cloudflare

**(a) DNS API with `proxied`: TRUE.** Only A, AAAA and CNAME records can be proxied. The minimum token permissions are Zone:Zone:Read (to look up the zone ID) and Zone:DNS:Edit, which matches lego's documentation. Gotcha: a proxied record forwards only HTTP/S. Non-HTTP ports such as SSH or database ports need a DNS-only record. Sources: https://developers.cloudflare.com/dns/manage-dns-records/reference/proxied-dns-records/ and https://go-acme.github.io/lego/dns/cloudflare/

**(b) Remotely-managed Tunnel via API: TRUE.** The steps are:

1. `POST /accounts/{acct}/cfd_tunnel` with `{"name":…, "config_src":"cloudflare"}`. The response includes `id` and `token`.
2. `PUT /accounts/{acct}/cfd_tunnel/{id}/configurations` with `{"config":{"ingress":[…,{"service":"http_status:404"}]}}`. A catch-all rule is mandatory.
3. Create a proxied DNS CNAME pointing to `<id>.cfargotunnel.com`.
4. Run `cloudflared tunnel --no-autoupdate run --token <T>`.

The token needs Account:Cloudflare Tunnel:Edit, plus Zone:DNS:Edit for the CNAME. The tunnel token can be fetched again later.

Gotchas:
- The PUT replaces the whole ingress list, so the control plane must render the full list every time.
- Treat the tunnel token as a secret, because anyone holding it can run the connector.

Source: https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/get-started/create-remote-tunnel-api/

**(c) R2 S3 endpoint: TRUE.** The endpoint is `https://<ACCOUNT_ID>.r2.cloudflarestorage.com` and the region is `auto`. An empty region or `us-east-1` is aliased to `auto`. R2 does not implement SSE headers, object tagging or object-lock headers on PutObject. Gotcha: path-style addressing works, and restic's defaults are fine. Source: https://developers.cloudflare.com/r2/api/s3/api/

**(d) Proxy behaviour:**
- **WebSockets: TRUE.** They work on all plans. Cloudflare closes idle connections and may restart servers during deploys, so the agent needs ping/pong heartbeats and reconnect with backoff. Source: https://developers.cloudflare.com/network/websockets/
- **gRPC: TRUE (needs the dashboard toggle).** It is available on all plans but requires port 443, TLS with HTTP/2 advertised over ALPN, and at least Full SSL mode. Tunnel public hostnames are not supported, Access does not cover gRPC, and the WAF inspects headers only. Source: https://developers.cloudflare.com/network/grpc-connections/
- **Client mTLS pass-through: TRUE (it does not pass through).** Cloudflare validates client certificates itself at the edge. The origin only receives certificate data as headers (`Cf-Client-Cert-*`, RFC 9440 `Client-Cert`, or via Workers). An agent's client certificate therefore cannot reach the control plane through the orange cloud. Source: https://developers.cloudflare.com/ssl/client-certificates/forward-a-client-certificate/

## 5. go-acme/lego: TRUE (with the v5 API change)

- **Version:** The module is `github.com/go-acme/lego/v5` (v5.7.0, 2026-11-21). The interface is `challenge.Provider{ Present(ctx, domain, token, keyAuth) error; CleanUp(ctx, domain, token, keyAuth) error }`.
- **Wiring:** Register providers with `client.Challenge.SetHTTP01Provider(p)` or `SetDNS01Provider(p)`. The HTTP path is `http01.ChallengePath(token)` = `/.well-known/acme-challenge/<token>`.
- **Cloudflare DNS provider:** `cloudflare.NewDNSProviderConfig(&Config{AuthToken, ZoneToken, …})`. The env vars are `CF_DNS_API_TOKEN` (DNS:Edit) and `CF_ZONE_API_TOKEN` (Zone:Read); one token holding both permissions is enough.
- **Custom HTTP-01 provider:** Store keyAuth by token in Postgres. Every node's proxy must route `/.well-known/acme-challenge/*` to the control plane, or have the agents serve it.
- **Wildcards:** They require DNS-01 (an ACME/Let's Encrypt rule).
- **Let's Encrypt limits** (page updated 2026-08-05):
  - 50 new certificates per registered domain per 7 days.
  - 5 certificates per exact identifier set per 7 days, with no override available.
  - 300 new orders per account per 3 hours.
  - 5 authorization failures per identifier per account per hour.
  - Renewals coordinated through ARI are exempt from all limits.
- **Gotchas:**
  - Implement ARI renewal.
  - Use LE staging in tests and CI.
  - A shared base domain such as `*.apps.example.com` hits the 50-per-week limit fast. Issue one wildcard certificate via DNS-01 instead of one certificate per app.
- **Sources:**
  - https://github.com/go-acme/lego/blob/v5.7.0/challenge/provider.go
  - https://go-acme.github.io/lego/dns/cloudflare/
  - https://letsencrypt.org/docs/rate-limits/

## 6. River: TRUE (pre-1.0)

- **Status:** Latest release v0.51.2 (2026-11-09). It uses pgx v5.11 through the `riverpgxv5` driver and is licensed MPL-2.0. The repo has about 5.9k stars and is actively released.
- **Transactional insert:** `InsertTx` makes a job visible only after the surrounding transaction commits.
- **Unique jobs (OSS):** `UniqueOpts{ByArgs, ByPeriod, ByQueue, ByState}`, enforced by a partial unique index.
- **Periodic jobs (OSS):** Defined with `NewPeriodicJob` using either an interval or a robfig/cron schedule. Only the elected leader inserts them, and `RunOnStart` is available. Schedules are kept in memory, so runs can be skipped. Durable periodic jobs, workflows and concurrency limits are paid Pro features.
- **Gotchas:**
  - Pin the River version and run `river migrate-up` (or the rivermigrate Go API) inside the app's migration step.
  - For user cron jobs, store `next_run_at` in our own table and enqueue with `ByPeriod` uniqueness rather than relying on OSS periodic jobs.
  - Breaking changes between 0.x minor versions are possible.
- **Sources:**
  - https://riverqueue.com/docs
  - https://riverqueue.com/docs/unique-jobs
  - https://riverqueue.com/docs/periodic-jobs
  - https://github.com/riverqueue/river

## 7. restic with S3 / R2: TRUE

- **Version:** Latest release v0.20.0 (2026-11-19).
- **Repository:** `restic -r s3:https://<acct>.r2.cloudflarestorage.com/<bucket> init` with `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_DEFAULT_REGION=auto`. A repository password is mandatory, and all data is always encrypted client-side.
- **Database dumps:** Use `backup --stdin-from-command --stdin-filename db.sql -- pg_dump …`. With this form, a non-zero exit from pg_dump cancels the backup and creates no snapshot. The docs discourage plain `--stdin` piping because it masks dump errors.
- **Retention:** `forget --keep-daily N … --prune`.
- **R2 issues:** No open restic bug was found. Issue #3757 ("Cloudflare R2 support") was closed in 2024, and a documentation PR (#21856) is open.
- **Gotchas:**
  - Losing the restic password loses the data, so store it outside the backed-up database.
  - Run `forget --prune` from a single job so it never runs concurrently with itself, because it takes an exclusive lock.
  - R2 lacks object-lock support via the S3 API, so there is no immutable or append-only protection. If ransomware resistance matters, use separate credentials for pruning.
- **Sources:**
  - https://restic.readthedocs.io/en/stable/040_backup.html
  - https://restic.readthedocs.io/en/stable/030_preparing_a_new_repo.html
  - https://github.com/restic/restic/issues/3757

## 8. Docker published ports bypass UFW: TRUE (still, as of Docker 29.10)

- **Evidence:** Docker's documentation says published-port traffic "gets diverted before it goes through the ufw firewall settings". Under the iptables backend, the documented fix is rules in `DOCKER-USER`. That means an ESTABLISHED,RELATED accept first, followed by negated drops, matching `--ctorigdstport` via conntrack because DNAT has already happened.
- **nftables:** Added in Docker 29.0 as experimental, enabled with `"firewall-backend": "nftables"`. This backend has **no `DOCKER-USER` chain**. Instead you add your own nft table with a `forward` hook at a priority that runs before Docker's.
- **Gotchas:**
  - The simplest robust mitigation is to never publish app ports on 0.0.0.0. Bind to `127.0.0.1` or attach containers to the internal network used by the node's reverse proxy, and publish only the proxy's 80/443.
  - Detect the firewall backend at agent install time.
- **Sources:**
  - https://docs.docker.com/engine/network/packet-filtering-firewalls/
  - https://docs.docker.com/engine/network/firewall-iptables/
  - https://docs.docker.com/engine/network/firewall-nftables/

## 9. Ed25519 challenge-response over WebSocket: PARTLY

- **Evidence:** There is no established Go library for this pattern. It is built by hand from stdlib `crypto/ed25519` plus a WebSocket library.
- **Library status:**
  - `coder/websocket` (formerly nhooyr) is actively maintained: v1.8.15 (2026-06), last push 2026-11-25. It is context-aware and passes the Autobahn test suite.
  - `gorilla/websocket` is not archived, but it is low-activity: last release v1.5.3 (2024-06), last push 2025-03.
- **Recommendation:** Use `coder/websocket`. Because mTLS cannot pass through Cloudflare (see 4d), application-level signing is the right choice.
- **Protocol gotchas:**
  - The server sends a ≥32-byte random nonce with a short TTL.
  - The agent signs a domain-separated message containing a context string, the nonce, agent_id, server host and a timestamp. This prevents cross-protocol and replay attacks and binds the signature to the TLS endpoint.
  - Compare with `ed25519.Verify`.
  - Enforce read limits (`SetReadLimit`) and a handshake deadline.
- **Sources:**
  - https://github.com/coder/websocket
  - https://github.com/gorilla/websocket
  - https://pkg.go.dev/crypto/ed25519

## Ranked recommendations

1. Agent transport: `coder/websocket` with Ed25519 challenge-response. Rejected alternatives are gRPC (blocked by Cloudflare Tunnel and Access) and mTLS (does not pass Cloudflare).
2. Certificates: lego v5, using a DNS-01 wildcard per base domain plus custom HTTP-01 for custom domains, with ARI renewal.
3. Git providers: GitHub App via manifest, GitLab OAuth with PKCE, and one shared Gitea/Forgejo adapter.
4. Backups: restic `--stdin-from-command` writing to R2.
5. Firewall: never publish app container ports on 0.0.0.0. Firewall-chain rules are only a second layer.

## Limitations and unresolved questions

- The GHES `HOSTNAME/settings/apps/new` form URL is not explicitly documented. It needs testing against a real GHES instance (3.18+).
- The URL of the Cloudflare tunnel-token re-fetch endpoint was not separately verified; only the create response's `token` field was.
- Gitea's `v28.x` release tag numbering was observed but not investigated. Its effect on version-gating logic is unknown.
- Forgejo OAuth-token git clone is assumed to behave like Gitea, based on shared code. The Forgejo `basic.go` file was not fetched.
- The R2 "Bucket Locks" product, as a substitute for S3 object lock, was not evaluated.
- The exact Cloudflare WebSocket idle timeout value was not captured; use a heartbeat of 30 s or less.
