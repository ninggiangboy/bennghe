# Research 01: Infra Runtime Fact-Check (Swarm, WireGuard, Traefik, Registry, Railpack, BuildKit)

Date: 2026-11-26. Method: official docs fetched live, then cross-checked against Go source on GitHub
(moby `docker-v29.10.1`, swarmkit `master`/`v2.1.2`, traefik `master`, buildkit `master`, wireguard-tools).
Where docs and source disagree, source wins and the conflict is flagged.

## Bottom line

The architecture is sound on every point checked. Six facts in the brief need to be corrected, and the plan
must account for them:

1. The overlay option `com.docker.network.driver.mtu` sets the **underlay** MTU. Docker subtracts the VXLAN
   overhead itself, so set it to the wg0 MTU (1420). Do not set it to wg0 minus 50.
2. Swarm configs are limited to **1000 KB**, not 500 KB (the docs are stale). Secrets are limited to 500 KB.
3. Traefik OSS **cannot** share ACME state across instances. The control plane should own ACME and push
   certs to Traefik through the file provider.
4. Traefik's file watcher only sees files **inside the watched directory**. A cert file referenced from
   elsewhere will not hot-reload.
5. Rootless buildkitd **cannot run as a Swarm service** because Swarm has no `systempaths=unconfined`.
   Run it as a plain container on the build node.
6. Docker 29's Go SDK lives at `github.com/moby/moby/client`, which is pre-1.0 (v0.6.x). Pin it.

## Answers

### 1. Swarm still maintained? TRUE (with a vendor caveat)
- moby ships Swarm in every Engine release. The latest are 29.10.0 (2026-11-16) and 29.10.1 (2026-11-24).
  swarmkit commits were still landing in Nov 2026, including a security merge from a fork on 2026-11-03.
  The last swarmkit tag is v2.1.2 (2026-04-23); moby vendors by commit.
- On 2025-07-01 Mirantis announced "extending support for Swarm for another five years" (about 2030) through MKE.
  **Risk:** IREN completed its acquisition of Mirantis on 2026-08-04. No Swarm deprecation has been
  announced, but whoever funds the work could change.
- Sources: https://github.com/moby/swarmkit/commits/master ,
  https://www.mirantis.com/company/press-center/company-news/mirantis-extends-swarm-support-another-five-years/ ,
  https://www.globenewswire.com/news-release/2026/08/04/3338169/0/en/IREN-Completes-Acquisition-of-Mirantis.html
- Gotcha: Docker 29 requires clients on API v1.44 or later. Use the new modules
  `github.com/moby/moby/client` + `github.com/moby/moby/api`; `docker/docker` is no longer the import path.
  https://docs.docker.com/engine/release-notes/29/ , https://github.com/moby/moby/tree/master/client

### 2. replicated-job / global-job: TRUE
- **Version:** Engine API **1.41** (Docker 20.10), where `POST /services/create` began accepting `ReplicatedJob`/`GlobalJob`.
  The service mode cannot be changed after creation. https://docs.docker.com/reference/api/engine/version-history/
- **Behavior:** a task that exits 0 is `Complete` and never restarted. Restart condition `any` behaves the same
  as `on-failure`. `--replicas` means total completions, and `--max-concurrent` sets parallelism.
  A global job runs once per matching node, including nodes added later. Update and rollback config are
  invalid for jobs. https://docs.docker.com/reference/cli/docker/service/create/
- **Go SDK:** `swarm.ServiceMode{ReplicatedJob: &swarm.ReplicatedJob{MaxConcurrent, TotalCompletions *uint64}}` or
  `GlobalJob: &swarm.GlobalJob{}`. Progress is reported in `Service.JobStatus` (JobIteration, LastExecution).
  **Gotcha:** the API comment says an empty `MaxConcurrent` defaults to **1**, while the CLI defaults it to `--replicas`.
  Always set both fields explicitly. https://github.com/moby/moby/blob/master/api/types/swarm/service.go
- **Cleanup:** there is no TTL or auto-removal (inferred: no field exists in the spec or docs). The agent must poll
  `JobStatus`/tasks and then call `ServiceRemove`. Re-running means a spec update (for example bumping
  `TaskTemplate.ForceUpdate`), which increments JobIteration. Set `RestartPolicy.MaxAttempts` so a failing job
  does not retry forever.

### 3. Swarm over WireGuard: TRUE, with MTU corrections
- **Addresses:** use `--advertise-addr <wg-ip>` for control and gossip and `--data-path-addr <wg-ip>` for VXLAN.
  data-path defaults to advertise-addr. Also set `--listen-addr <wg-ip>:2377`, because the default is 0.0.0.0.
  https://docs.docker.com/engine/swarm/networking/
- **Ports:** 2377/tcp (managers), 7946/tcp+udp, and 4789/udp. Allow them on `wg0` only and DROP them on the public NIC.
  The VXLAN socket listens on all interfaces, and Docker's tutorial already warns about unencrypted 4789.
  https://docs.docker.com/engine/swarm/swarm-tutorial/
- **wg0 MTU:** wg-quick defaults to the route MTU minus 80, which gives **1420** on a 1500 link
  (https://github.com/WireGuard/wireguard-tools/blob/master/src/wg-quick/linux.bash). Pin `MTU = 1420` explicitly,
  or lower if any provider's underlay is under 1500.
- **Overlay MTU (key correction):** in the moby source, `maxMTU()` takes `mtu = opt or 1500`, subtracts 50 for IPv4
  transport (70 for IPv6 or unknown), and subtracts more if `encrypted`. The option is therefore the
  **transport** MTU. Set `com.docker.network.driver.mtu=1420` and containers get 1370.
  Leaving it at the default (1500) gives containers 1450, which produces oversized packets inside wg0 and
  causes fragmentation or blackholing.
  https://github.com/moby/moby/blob/docker-v29.10.1/daemon/libnetwork/drivers/overlay/encryption.go (TestMaxMTU in the same dir)
- Gotchas:
  - Recreate the `ingress` network with the same MTU option (as in the docs' MTU=1200 example), or disable the routing mesh.
  - Do not use `--opt encrypted`, because WireGuard already encrypts and IPsec would double the overhead.
  - Order systemd so docker starts `After=wg-quick@wg0`.
  - The nftables firewall backend has not migrated overlay rules ("not yet migrated from iptables"), so stay on iptables.

### 4. start-first + rollback + monitor vs HEALTHCHECK: TRUE (Swarm waits for healthy)
- The executor's `controller.Start()` returns, and the task becomes RUNNING, only after a `health_status: healthy`
  event. An `unhealthy` result stops the container and fails the task. With **no healthcheck** (or `NONE`),
  the task is RUNNING as soon as the process starts.
  https://github.com/moby/moby/blob/docker-v29.10.1/daemon/cluster/executor/container/controller.go
- The swarmkit updater waits until the task state is RUNNING or later. Under start-first it stops the old task only
  after the new one is RUNNING, then watches for failures during `monitor` (default 5s). If the failure ratio exceeds
  `max_failure_ratio` (default 0), it applies `failure_action`.
  https://github.com/moby/swarmkit/blob/master/manager/orchestrator/update/updater.go
- Gotchas:
  - Without a healthcheck, "success" only means the process did not crash within `monitor`. Inject a default
    healthcheck or raise `monitor` (for example to 30s).
  - Also set `rollback_config.order=start-first`, because it defaults to stop-first.
  - Traefik uses task IPs and polls every 15s by default. During rollouts, either set the label
    `traefik.swarm.lbswarm=true` (Swarm VIP) or lower `refreshSeconds` to avoid routing to removed tasks.

### 5. Traefik v3: separate `providers.swarm`: TRUE. Shared ACME HA in OSS: FALSE. File-provider hot reload: PARTLY
- `providers.swarm` is a separate provider in v3. It reads **service** labels (compose `deploy.labels`), requires
  `traefik.http.services.<n>.loadbalancer.server.port`, defaults `refreshSeconds` to 15s, and needs a manager-node
  socket (or TCP API). https://doc.traefik.io/traefik/reference/install-configuration/providers/swarm/
- On ACME the docs say: "it is not possible to run multiple instances of Traefik 2.0 with Let's Encrypt enabled".
  The KV store was dropped in 2.0, and for HA they recommend Traefik Enterprise or cert-manager (Kubernetes).
  `acme.json` is local per instance.
  https://doc.traefik.io/traefik/reference/install-configuration/tls/certificate-resolvers/acme/
- `[[tls.certificates]]` in file-provider dynamic config is hot-added and removed, and `providers.file.watch`
  uses fsnotify. **Gotcha (from source):** the watcher only covers `directory` and its direct entries. Cert
  files referenced by path elsewhere are read at load time and **not watched**.
  Fix: put PEM inline in the YAML (`certFile` accepts content), or write certs into the watched dir and then
  atomically rename the YAML. Bind-mount the parent dir, not single files (fsnotify caveat).
  https://doc.traefik.io/traefik/reference/install-configuration/providers/others/file/ ,
  https://github.com/traefik/traefik/blob/master/pkg/provider/file/file.go
- Recommendation: the control plane runs ACME and Traefik only consumes certs. For HTTP-01, route
  `/.well-known/acme-challenge/` from every Traefik to the control plane; otherwise use DNS-01.

### 6. Distribution registry: TRUE
- GC is stop-the-world: "ensure that the registry is in read-only mode or not running at all". Restart with
  `maintenance.readonly.enabled: true`, run `registry garbage-collect [--delete-untagged] config.yml`, then restart
  without it. https://distribution.github.io/distribution/about/garbage-collection/ ,
  https://distribution.github.io/distribution/about/configuration/
- Manifest delete: enable `storage.delete.enabled: true` (`REGISTRY_STORAGE_DELETE_ENABLED=true`). Then call
  `DELETE /v2/<name>/manifests/<digest>`, which must be a **digest** (HEAD the tag with the right Accept header to get
  `Docker-Content-Digest`). Without the setting it returns 405. Disk is freed only by the next GC.
  https://distribution.github.io/distribution/spec/api/ . The current release is v3.1.2 (2026-09-24).
- CA: `/etc/docker/certs.d/<host:port>/ca.crt`, where "You do not need to restart Docker"
  (https://distribution.github.io/distribution/about/insecure/). Source confirms both the classic and the
  containerd image stores (the default for fresh 29.x installs) read certs.d per request (`daemon/hosts.go`).
  **Every Swarm node pulls images itself, so every node needs the CA.**

### 7. Railpack vs Nixpacks: TRUE (Nixpacks in maintenance mode)
- The Nixpacks README says: "Maintenance Mode ... not under active development. We recommend using Railpack".
  Railpack is MIT-licensed and active: v0.43.1 on 2026-11-23, about 1.3k stars.
  https://github.com/railwayapp/nixpacks , https://github.com/railwayapp/railpack
- Railpack is built on BuildKit. `railpack build` needs a BuildKit daemon (`BUILDKIT_HOST`). For production the docs
  "highly recommend" the frontend:
  - `railpack prepare <dir> --plan-out railpack-plan.json --info-out railpack-info.json`
    (exit code 75 means retryable).
  - Then `buildctl build --frontend=gateway.v0 --opt source=ghcr.io/railwayapp/railpack-frontend --local context=... --local dockerfile=<dir with plan>`.
  - Pin the frontend image to the same version as the CLI.
  - Secrets: pass names to `prepare --env` and values as `--secret id=X,env=X`, plus a `secrets-hash` build arg.
  - Use the `cache-key` build arg for tenant cache isolation.
  https://railpack.com/platforms/running-railpack-in-production

### 8. Rootless buildkitd in a container: TRUE, but not as a Swarm service
- Run `moby/buildkit:rootless` with `seccomp=unconfined`, `apparmor=unconfined`, and `systempaths=unconfined`
  (or `--privileged`, described as "almost safe for rootless"). overlayfs needs kernel 5.11 or later.
  Ubuntu 24.04+ hosts need `kernel.apparmor_restrict_unprivileged_userns=0`.
  https://github.com/moby/buildkit/blob/master/docs/rootless.md
- **Gotcha:** Swarm `ContainerSpec.Privileges` supports Seccomp and AppArmor but has **no systempaths/privileged/devices**
  (https://github.com/moby/moby/blob/master/api/types/swarm/container.go). Inside Swarm you would need
  `--oci-worker-no-process-sandbox`, which the docs discourage. Have the build-node agent run buildkitd via the
  container API instead.
- Custom-CA push: buildkitd does **not** use Docker's certs.d. Add `[registry."host:port"] ca=["/path/ca.pem"]` to
  `buildkitd.toml` (rootless path `~/.config/buildkit/buildkitd.toml`, read at startup, so changing the CA needs a
  restart). Optionally restrict `[frontend."gateway.v0"] allowedRepositories` to the railpack frontend.
  https://github.com/moby/buildkit/blob/master/docs/buildkitd.toml.md

### 9. Stateful services: TRUE (stop-first is an engineering inference)
- Constraints support `node.id`, `node.hostname`, and `node.labels.*`. Only managers can set node labels, which makes
  them the safe choice, unlike `engine.labels`. Volumes "outlive tasks and services" and are created on the node
  where the task lands. https://docs.docker.com/reference/cli/docker/service/create/ ,
  https://docs.docker.com/engine/swarm/services/
- Prefer a per-node label (for example `node.labels.paas.node==<uuid>`) over `node.id`, because the ID changes if
  a node leaves and rejoins.
- `update_config.order=stop-first` is the Swarm default. Keep it for databases: start-first would run two engines
  on one data dir. Docs do not state this explicitly; it follows from lock semantics. Expect downtime during
  updates and no automatic failover when the node dies.

### 10. Configs/secrets: PARTLY
- Both are immutable, and only labels can change ("only updates to Labels are allowed"). Rotation means creating
  a new name and updating the service. You cannot remove one that is in use.
  https://docs.docker.com/engine/swarm/configs/ , https://docs.docker.com/engine/swarm/secrets/
- Size limits (source): secrets must be under 500 KB (`MaxSecretSize = 500*1024`). Configs must be under
  **1000 KB** (`MaxConfigSize = 1000*1024`, also in the 29.10.1 vendor tree), while the docs still say 500 KB.
  Design for 500 KB to be safe. https://github.com/moby/swarmkit/blob/master/manager/controlapi/config.go
- API readability: config `Spec.Data` is returned by inspect. Secret `Spec.Data` is always nil'd ("never returned").
  Both work only through a **manager** API. Keep plaintext secrets in the control plane's encrypted store and treat
  Swarm as write-only. https://github.com/moby/swarmkit/blob/master/manager/controlapi/secret.go

## Adoption risk summary

| Component | Maturity | Activity (Nov 2026) | Main risk |
|---|---|---|---|
| Swarm (moby/swarmkit) | High, feature-frozen | Monthly Engine releases | Funding/ownership (Mirantis to IREN); few new features |
| WireGuard | High | Stable | MTU misconfiguration across providers |
| Traefik v3 | High | v3.8.0, 2026-11-23 | No OSS ACME HA, so ACME must live in the control plane |
| distribution v3 | High | v3.1.2, 2026-09 | Stop-the-world GC needs a maintenance window |
| Railpack | Medium (0.x) | Weekly releases | Pre-1.0 plan format and frontend coupling; pin versions |
| BuildKit rootless | High | v0.35.1, 2026-11-19 | Host sysctl/AppArmor prerequisites per distro |
| moby Go client | Medium (v0.6.x) | Active | Pre-1.0 module, so API churn |

## Limitations
- Nothing was tested live: no cluster or MTU path tests across real VPS providers. Some providers (for example
  ones with 1450 underlay or PPPoE) need wg0 below 1420, so verify with `ping -M do -s` in phase 3.
- Did not check Traefik's behavior with `allowEmptyServices` / unhealthy Swarm tasks in depth, or BuildKit's
  rootless performance.
- Job auto-cleanup "does not exist" is inferred from the absence of any spec field or docs mention, not from an
  explicit statement.

## Unresolved questions
1. Is the cluster IPv4-only on wg0? With IPv6 transport the overlay overhead is 70 bytes, giving 1350 inside containers.
2. ACME challenge type: HTTP-01 (route challenge paths to the control plane) or DNS-01 (needs DNS provider credentials)?
3. Should the registry GC window take the registry read-only per cluster on a schedule, or run on demand, given
   that builds and pushes fail during it?
