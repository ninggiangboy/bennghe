---
phase: 5
title: "Build & Deploy"
status: pending
priority: P1
effort: "4w"
dependencies: [4]
---

# Phase 5: Build & Deploy

## Goal
Deploy app từ image, Dockerfile hoặc mã nguồn (Railpack) thành Swarm service có replica, HTTPS, env/secret, rolling update chờ healthcheck, tự rollback, rollback tay trả lại **toàn bộ cấu hình** của bản trước; các deployment của cùng một app chạy tuần tự; mọi workload đi qua chính sách kiểm duyệt và được cách ly theo project. Đây là mốc **alpha2**. Ở phase này nguồn Git là URL `https://` công khai; tích hợp provider ở phase 6.

## Requirements
- **`servicespec` là chủ sở hữu duy nhất** của: network, route (qua `routing` phase 4), mount healthcheck/envexec, `update_config`/`rollback_config`, đặt tên secret, constraint `bn.ready`, chính sách kiểm duyệt. App, database (phase 7), job (phase 7–8), stack (phase 9) và dịch vụ hạ tầng (Traefik, cloudflared) đều sinh spec qua đây. <!-- Red Team: N -->
- **Chính sách kiểm duyệt workload** (`servicespec/admission.go`): mặc định từ chối bind mount (trừ binary của platform), volume ngoài project, `privileged`, `cap_add`, devices, host network/PID, placement ngoài nhãn được phép của project, label `traefik.*` do người dùng tự viết. Cờ `privileged` cho từng app/stack chỉ admin bật, có audit. <!-- Red Team: E, F -->
- **Cách ly project:** mỗi project trong mỗi cụm có overlay riêng `bn-proj-<id>` (không attachable); Traefik được gắn vào các network này (phase 4). <!-- Red Team: E -->
- **Registry mỗi cụm:** <!-- Red Team: J, E -->
  - Container `distribution` thường do agent trên build node quản lý (không phải Swarm service), bind vào `<wg-ip>:5000` — không bao giờ nghe trên IP public.
  - Tên ổn định `registry.<cluster>.bn.internal`, agent ghi vào `/etc/hosts` mọi node; cert TLS do CA nội bộ của cụm cấp cho tên này; CA cài ở `/etc/docker/certs.d/registry.<cluster>.bn.internal:5000/ca.crt` mọi node (không cần restart Docker). Chuyển vai trò build sang node khác giữ nguyên tên; tài liệu hoá việc chuyển volume.
  - Xác thực **token auth** của registry: control plane làm token server, cấp quyền `pull` theo project cho node chạy workload của project đó và quyền `push` chỉ cho build node.
- **Build:** buildkitd rootless chạy như **container thường** do agent trên build node quản lý (Swarm service không có `systempaths=unconfined`); CA registry nằm trong `buildkitd.toml` (BuildKit không đọc `certs.d`; đổi CA thì restart buildkitd); `cache-key` riêng cho mỗi project và `allowedRepositories` ghim theo research-01 mục 8. Giới hạn RAM/CPU cấu hình được. <!-- Red Team: E -->
  - Clone bằng credential ngắn hạn truyền qua `GIT_ASKPASS` (không nhúng token vào URL); `protocol.allow` chỉ cho https; log build lọc mọi chuỗi giống token. <!-- Red Team: M -->
  - Có Dockerfile thì build Dockerfile; không có thì `railpack prepare` rồi build qua frontend `ghcr.io/railwayapp/railpack-frontend` ghim cùng phiên bản CLI.
  - Push xong ghi **digest**; mọi service spec dùng `image@sha256:...`, không dùng tag. <!-- Red Team: J -->
- **App:** `apps(project_id, cluster_id, source_type, image_ref?, git_url?, branch, build_config, port, healthcheck{type, path, interval, timeout}, replicas, resources, placement (trong giới hạn project), privileged)`; env nằm ở bảng riêng `app_env(key, value_enc, is_secret)`.
- **Deployment tuần tự theo app:** <!-- Red Team: A -->
  - Tại mỗi thời điểm chỉ một deployment của một app ở `rolling_out` (River unique job theo `app_id` + advisory lock Postgres).
  - Deployment mới vào hàng đợi huỷ các deployment cũ hơn chưa tới `rolling_out` (trạng thái `superseded`).
  - Không gửi cập nhật khi `UpdateStatus.State` của service đang `updating`.
  - Không tự deploy commit cũ hơn commit đang chạy trên cùng branch, trừ khi người dùng bấm tay.
- **Snapshot deployment:** mỗi deployment lưu toàn bộ đầu vào đã render (digest image, env tham chiếu, tên + hash secret, resources, healthcheck, route). Rollback tay = tạo deployment mới áp lại snapshot của deployment thành công trước đó. Secret có hậu tố hash được giữ nếu còn được tham chiếu bởi 5 deployment thành công gần nhất; cũ hơn thì dọn. <!-- Red Team: A -->
- **State machine:** `queued → cloning → building → pushing → rolling_out → healthy | failed | rolled_back | superseded`; mỗi bước có timeout.
- **Rolling update:** `update_config{order: start-first, failure_action: rollback, monitor: 30s, parallelism: 1, delay: 10s}`, `rollback_config.order=start-first` tường minh, `stop_grace_period: 30s`. Swarm chờ HEALTHCHECK đạt mới coi task mới là running (research-01 mục 4).
- **Swarm tự rollback là kết thúc của revision đó:** khi agent báo `rolled_back`, control plane trong cùng transaction đặt desired state của app về snapshot thành công trước đó, để reconciler không áp lại bản hỏng. <!-- Red Team: A -->
- **Healthcheck và envexec:** binary tĩnh `/opt/bennghe/bin/healthcheck` và `/opt/bennghe/bin/envexec` có trên mọi node (điều kiện của `bn.ready`), mount read-only. Env đánh dấu secret được đưa vào dạng Swarm secret; `envexec` đọc file secret thành biến môi trường rồi `exec` entrypoint gốc của image (đọc từ image config). Env thường vẫn nằm trong service spec và UI ghi rõ là không bí mật. <!-- Red Team: M -->
- **Log:** build log nén trong Postgres (tối đa 5 MB/deployment); runtime log qua `ServiceLogs` từ agent leader, đi qua control plane tới UI bằng WebSocket. Xem log là action đọc nhạy cảm (phase 1).
- **Image từ registry ngoài:** `registry_credentials` mã hoá.
- **Dọn image:** luôn giữ image đang chạy, image trong `PreviousSpec` của Swarm và 5 deployment **thành công** gần nhất; deployment lỗi tính riêng (giữ 3). Xoá manifest theo digest (`REGISTRY_STORAGE_DELETE_ENABLED`); GC cần registry read-only nên build/push của cụm bị hoãn trong lúc GC: chạy theo lịch hằng tuần trong cửa sổ bảo trì cấu hình được (mặc định Chủ nhật 03:00 theo timezone của cụm) và có nút chạy ngay; deployment trong lúc GC xếp hàng. <!-- Updated: Validation Session 1 - GC theo lịch + bấm tay --> <!-- Red Team: J -->
- **Kéo trước image:** sau khi deployment `healthy`, chạy global job kéo image hiện tại lên mọi node đủ điều kiện, để việc dời task khi build node chết không cần registry. <!-- Red Team: J -->
- **RBAC/audit/sự kiện:** route app, env, deploy, rollback, log, registry credentials có `x-permission` + audit; phát `deploy.succeeded|failed|rolled_back` qua `events.Emit`. <!-- Red Team: C -->
- **UI:** wizard tạo app, trang app (deployments, logs, env có cờ secret, domains, settings), Deploy và Rollback.

## Files to Create / Modify
- Create: `internal/cp/servicespec/` (app, job, infra, admission, secrets naming), `internal/cp/apps/`, `internal/cp/deploy/` (state machine, tuần tự hoá, snapshot), `internal/cp/registry/` (CA cụm, token server, dọn image)
- Create: `internal/agent/build/` (clone với GIT_ASKPASS, Dockerfile/Railpack, BuildKit client, quản lý container buildkitd + `buildkitd.toml`), `internal/agent/registryhost/` (container distribution, `/etc/hosts`, CA), `internal/agent/services/` (ensure service với version index, theo dõi rollout, logs)
- Modify: `cmd/healthcheck/`, `cmd/envexec/`, `internal/agent/nodeprep/`
- Create: `migrations/<timestamp>_apps_deployments.sql`
- Modify: `proto/agent/v1/agent.proto`, `api/openapi.yaml`, `internal/cp/rbac/policy.go`, `web/src/routes/apps/*`

## Tasks & Steps
1. `servicespec` + chính sách kiểm duyệt + network theo project (golden test).
2. CA cụm, registry container bind WG, `/etc/hosts`, token server.
3. buildkitd container + `buildkitd.toml`, build client, GIT_ASKPASS + lọc log, Railpack, digest.
4. healthcheck + envexec; secret env.
5. Ensure service với version index; theo dõi rollout; xử lý `rolled_back`.
6. Deployment tuần tự + supersede + snapshot + rollback tay.
7. Log build/runtime.
8. Dọn image có bảo vệ; kéo trước image.
9. RBAC/audit/sự kiện; UI.

## Todo
- [ ] servicespec + admission + network project
- [ ] Registry: CA, container bind WG, tên ổn định, token server
- [ ] buildkitd + build + GIT_ASKPASS + Railpack + digest
- [ ] healthcheck + envexec + secret env
- [ ] Ensure service + xử lý rolled_back
- [ ] Tuần tự hoá + snapshot + rollback tay
- [ ] Log
- [ ] Dọn image có bảo vệ + kéo trước image
- [ ] RBAC/audit/sự kiện cho route mới
- [ ] UI app

## Success Criteria
- Deploy 3 app mẫu (Go Dockerfile, Node qua Railpack, `nginx` Docker Hub) đều có HTTPS.
- k6 trong lúc rolling update `replicas=3`: 0 request non-2xx.
- Healthcheck luôn lỗi: Swarm rollback, bản cũ vẫn phục vụ, deployment `rolled_back`, reconciler không áp lại bản hỏng (theo dõi 10 phút).
- Push A rồi B cách 20 giây: A thành `superseded` hoặc chạy xong trước B; không bao giờ A ghi đè B.
- Đổi env sai rồi Rollback: bản trước chạy với env cũ.
- Developer deploy app mount `/var/run/docker.sock` hoặc tự viết label `traefik.*`: bị từ chối.
- App project A không kết nối được tới service project B cùng cụm.
- Tắt build node: task bị dời vẫn khởi động được nhờ image đã kéo trước.
- Build trên build node 1 GB RAM không làm app khác trên node đó bị OOM-kill.

## Verification
- Golden test `servicespec` và admission; lab Incus: deploy, k6, healthcheck lỗi, push đồng thời, cách ly network, tắt build node.

## Risks
- Image có mount `noexec` hoặc entrypoint lạ làm `envexec`/healthcheck lỗi. Phản ứng: cho tắt healthcheck của platform và dùng HEALTHCHECK của image; cho phép secret ở dạng file thay vì env.
- Token server registry là phần tự viết theo spec Docker token auth. Tín hiệu: node không pull được. Phản ứng: test tích hợp với `distribution` thật trong CI.
- Registry chết thì không deploy được bản mới; task đang chạy và task dời có image kéo trước vẫn chạy. HA registry ngoài phạm vi v1.
