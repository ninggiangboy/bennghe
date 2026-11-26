---
phase: 5
title: "Build & Deploy"
status: pending
priority: P1
effort: "7w"
dependencies: [4]
---

# Phase 5: Build & Deploy

## Goal
Deploy app từ image, Dockerfile hoặc mã nguồn (Railpack) thành Swarm service có replica, HTTPS, env/secret, rolling update chờ healthcheck, tự rollback, rollback tay trả lại **toàn bộ cấu hình** của bản trước; các deployment của cùng một app chạy tuần tự; mọi workload đi qua chính sách kiểm duyệt và được cách ly theo project. Đây là mốc **alpha2**. Ở phase này nguồn Git là URL `https://` công khai; tích hợp provider ở phase 6.

## Requirements
- **`servicespec` là chủ sở hữu duy nhất** của: network, route (qua `routing` phase 4), mount healthcheck/envexec, `update_config`/`rollback_config`, đặt tên secret, constraint `bn.ready`, chính sách kiểm duyệt. App, database (phase 7), job (phase 7–8), stack (phase 9) và dịch vụ hạ tầng (Traefik, cloudflared) đều sinh spec qua đây. <!-- Red Team: N -->
- **Chính sách kiểm duyệt workload** (`servicespec/admission.rs`; route bật `privileged` và xem giá trị secret có `reauth: true` theo phase 1 <!-- Red Team (Dokploy): 9 -->): mặc định từ chối bind mount (trừ binary của platform), volume ngoài project, `privileged`, `cap_add`, devices, host network/PID, placement ngoài nhãn được phép của project, label `traefik.*` do người dùng tự viết. Cờ `privileged` cho từng app/stack chỉ admin bật, có audit. <!-- Red Team: E, F -->
- **Cách ly project:** mỗi project trong mỗi cụm có overlay riêng `bn-proj-<id>` (không attachable); Traefik được gắn vào các network này (phase 4). <!-- Red Team: E -->
- **Registry mỗi cụm:** <!-- Red Team: J, E -->
  - Container `distribution` thường do agent trên build node quản lý (không phải Swarm service), bind vào `<wg-ip>:5000` — không bao giờ nghe trên IP public.
  - Tên ổn định `registry.<cluster>.bn.internal`, agent ghi vào `/etc/hosts` mọi node; cert TLS do CA nội bộ của cụm cấp cho tên này; CA cài ở `/etc/docker/certs.d/registry.<cluster>.bn.internal:5000/ca.crt` mọi node (không cần restart Docker). Chuyển vai trò build sang node khác giữ nguyên tên; tài liệu hoá việc chuyển volume.
  - Xác thực **token auth** của registry: control plane làm token server, cấp quyền `pull` theo project cho node chạy workload của project đó và quyền `push` chỉ cho build node.
- **Build:** buildkitd rootless chạy như **container thường** do agent trên build node quản lý (Swarm service không có `systempaths=unconfined`); CA registry nằm trong `buildkitd.toml` (BuildKit không đọc `certs.d`; đổi CA thì restart buildkitd); `cache-key` riêng cho mỗi project và `allowedRepositories` ghim theo research-01 mục 8. Giới hạn RAM/CPU cấu hình được. <!-- Red Team: E -->
  - Agent không có thư viện BuildKit client bằng Rust đủ tin cậy (`buildkit-client` mới 0.1.4, ~470 lượt tải), nên chạy **`buildctl` bên trong container `moby/buildkit`** (cùng image với buildkitd, ghim phiên bản) qua `bollard` exec: `buildctl build --progress rawjson` (đã xác nhận có trong source BuildKit) cho luồng tiến trình có cấu trúc để stream log và nhận biết bước; huỷ = kết thúc tiến trình exec. Cách này giống cách agent đã gọi `git`, `railpack` và `docker stack deploy`. Cần kiểm trong lab: ngữ nghĩa huỷ giữa chừng và dọn thư mục tạm. <!-- Updated: Rust 2026-11-26 -->
  - Clone bằng credential ngắn hạn truyền qua `GIT_ASKPASS` (không nhúng token vào URL); `protocol.allow` chỉ cho https; log build lọc mọi chuỗi giống token. <!-- Red Team: M -->
  - Có Dockerfile thì build Dockerfile; không có thì `railpack prepare` rồi build qua frontend `ghcr.io/railwayapp/railpack-frontend` ghim cùng phiên bản CLI.
  - Push xong ghi **digest**; mọi service spec dùng `image@sha256:...`, không dùng tag. <!-- Red Team: J -->
- **App:** `apps(project_id, cluster_id, source_type, image_ref?, git_url?, branch, build_config, port, healthcheck{type, path, interval, timeout}, replicas, resources, placement (trong giới hạn project), privileged)`; env nằm ở bảng riêng `app_env(key, value_enc, is_secret)`.
- **Deployment tuần tự theo app:** <!-- Red Team: A -->
  - Tại mỗi thời điểm chỉ một deployment của một app ở `rolling_out` (job duy nhất theo `app_id` bằng `job_key` + advisory lock Postgres).
  - Deployment mới vào hàng đợi huỷ các deployment cũ hơn chưa tới `rolling_out` (trạng thái `superseded`).
  - Không gửi cập nhật khi `UpdateStatus.State` của service đang `updating`.
  - Không tự deploy commit cũ hơn commit đang chạy trên cùng branch, trừ khi người dùng bấm tay.
- **Snapshot deployment:** mỗi deployment lưu toàn bộ đầu vào đã render (digest image, `source_sha`, **giá trị đã giải** của env không bí mật kể cả biến lấy từ `${{project.KEY}}`, tên + hash Swarm secret cho giá trị bí mật, resources, healthcheck, route). Rollback áp lại đúng các giá trị này, không giải lại tham chiếu. <!-- Red Team (Dokploy): 14 --> Rollback tay = tạo deployment mới áp lại snapshot của deployment thành công trước đó. Secret có hậu tố hash được giữ nếu còn được tham chiếu bởi 5 deployment thành công gần nhất; cũ hơn thì dọn. <!-- Red Team: A -->
- **State machine:** `queued → cloning → building → pushing → rolling_out → healthy | failed | rolled_back | superseded | cancelled`; mỗi bước có timeout. <!-- Updated: Dokploy comparison 2026-11-26 - thêm cancelled -->
- **Rolling update:** `update_config{order: start-first, failure_action: rollback, monitor: 30s, parallelism: 1, delay: 10s}`, `rollback_config.order=start-first` tường minh, `stop_grace_period: 30s`. Swarm chờ HEALTHCHECK đạt mới coi task mới là running (research-01 mục 4).
- **Swarm tự rollback là kết thúc của revision đó:** khi agent báo `rolled_back`, control plane trong cùng transaction đặt desired state của app về snapshot thành công trước đó, để reconciler không áp lại bản hỏng. <!-- Red Team: A -->
- **Healthcheck và envexec:** binary tĩnh `/opt/bennghe/bin/healthcheck` và `/opt/bennghe/bin/envexec` có trên mọi node (điều kiện của `bn.ready`), mount read-only. Env đánh dấu secret được đưa vào dạng Swarm secret; `envexec` đọc file secret thành biến môi trường rồi `exec` entrypoint gốc của image (đọc từ image config). Env thường vẫn nằm trong service spec và UI ghi rõ là không bí mật. <!-- Red Team: M -->
- **Log:** build log nén trong Postgres (tối đa 5 MB/deployment); runtime log qua `ServiceLogs` từ agent leader, đi qua control plane tới UI bằng WebSocket. Xem log là action đọc nhạy cảm (phase 1).
- **Image từ registry ngoài:** `registry_credentials` mã hoá.
- **Huỷ deployment:** thêm trạng thái `cancelling` và trạng thái kết thúc `cancelled`. Người có quyền deploy app huỷ được deployment ở `queued`, `cloning` hoặc `building`; khi upload manifest đã bắt đầu (`pushing`) thì không huỷ được nữa. Deployment ở `cancelling` **giữ slot build** cho tới khi agent xác nhận đã huỷ context clone/solve BuildKit và xoá thư mục tạm, hoặc tới khi hết timeout của bước; sau đó mới thành `cancelled`. Mọi chuyển trạng thái là `UPDATE … WHERE state = <trạng thái mong đợi>`, nên kết quả đến trễ từ outbox không thể đưa deployment đã huỷ sang `rolling_out`. Huỷ có audit và phát `deploy.cancelled`. <!-- Updated: Dokploy comparison 2026-11-26 --> <!-- Red Team (Dokploy): 11 -->
- **Stream log build trực tiếp:** agent ghi toàn bộ log build ra file tạm trên đĩa (tối đa 5 MB, đã qua bộ lọc chuỗi giống token) và gửi file này kèm kết quả job (lớp không bao giờ bị bỏ của outbox phase 2) khi deployment kết thúc; đây là bản lưu chính thức, nén vào Postgres. Song song, agent đẩy log theo từng khối qua kênh WebSocket (best-effort); control plane ghi các khối vào bảng `build_log_chunks(deployment_id, seq, data)` và phát lại tới người đang xem (cùng action đọc nhạy cảm với log), nên mở trang giữa chừng hoặc sau khi control plane restart vẫn thấy phần đã có. Khi có bản chính thức thì xoá các khối. <!-- Updated: Dokploy comparison 2026-11-26 --> <!-- Red Team (Dokploy): 11 -->
- **Bộ điều phối build theo cụm và giới hạn song song:** `clusters.max_concurrent_builds` (mặc định 1). Một job duy nhất theo cụm chọn deployment `queued` kế tiếp bằng `ORDER BY` ưu tiên (production trước preview của phase 11, preview chờ quá 30 phút được xếp ngang) rồi `created_at`, `FOR UPDATE SKIP LOCKED`, chỉ khi số deployment ở `cloning|building|pushing|cancelling` nhỏ hơn giới hạn và registry không đang GC. Job này được enqueue lại ở mọi chuyển trạng thái kết thúc hoặc giải phóng slot, khi GC xong, và chạy định kỳ mỗi phút để phòng sót. UI hiện vị trí trong hàng theo cùng thứ tự. Build service của stack (phase 9) cũng đi qua bộ điều phối này. <!-- Updated: Dokploy comparison 2026-11-26 --> <!-- Red Team (Dokploy): 11 -->
- **Deployment không cần build không chiếm slot:** rollback tay, app `source_type=image` và redeploy cùng digest đi thẳng từ `queued` tới `rolling_out` (vẫn tuần tự theo app). <!-- Red Team (Dokploy): 11 -->
- **Timeout mỗi bước:** `cloning` 10 phút, `building` 30 phút, `pushing` 10 phút (cấu hình theo app); mất session agent của build node trong một bước quá 2 phút thì deployment thành `failed` và trả slot. <!-- Red Team (Dokploy): 11 -->
- **Dọn Docker trên từng node:** agent chạy dọn theo bộ hẹn giờ cục bộ hằng ngày lúc 03:30 theo timezone của cụm (chạy được khi control plane offline), và chạy ngay khi disk > 90% nhưng không quá một lần mỗi 30 phút: <!-- Red Team (Dokploy): 12 --> <!-- Red Team (Dokploy): 15 -->
  - xoá container đã dừng không thuộc task Swarm nào đang được theo dõi và không phải dấu chống chạy trùng của job (quy tắc giữ lại ở phase 7);
  - xoá image không được container nào dùng, **trừ** danh sách giữ lại gửi kèm desired state: digest hiện tại của mọi app/stack/preview trong cụm, image trong `PreviousSpec`, image đã kéo trước, ops image, image của mọi lịch cron, image hạ tầng (Traefik, cloudflared, registry, buildkitd); ngoài ra không bao giờ xoá image được kéo trong 60 phút gần nhất hoặc thuộc service đang rollout (phòng trường hợp danh sách giữ lại tới node trễ hơn lệnh cập nhật service);
  - trên build node, giới hạn cache BuildKit bằng gcpolicy trong `buildkitd.toml` với `maxUsedSpace = "20%"` và `keepDuration = "168h"`, ghim phiên bản BuildKit có hai khoá này (khoá cũ `gckeepstorage` không dùng);
  - không bao giờ xoá volume.
  Kết quả (dung lượng giải phóng, lỗi) gửi qua outbox; lỗi phát `node.cleanup_failed`. Dọn xong mà disk vẫn > 90% thì không chạy lại vòng lặp mà để cảnh báo `node.disk_high` của phase 2 báo. <!-- Updated: Dokploy comparison 2026-11-26 -->
- **Biến tham chiếu cấp project:** bảng `project_env(project_id, key, value_enc, is_secret)`. Giá trị env của app (và của stack ở phase 9, cron ở phase 8) được dùng cú pháp `${{project.KEY}}`; control plane giải bằng một bộ giải duy nhất `crates/cp/src/envref` lúc render snapshot. Khoá không tồn tại làm render thất bại với thông báo nêu tên khoá. Nếu biến project được tham chiếu là bí mật thì biến kết quả của app cũng được coi là bí mật (đi đường Swarm secret), bất kể cờ của app. Đổi biến project không tự redeploy. <!-- Updated: Dokploy comparison 2026-11-26 --> <!-- Red Team (Dokploy): 14 --> <!-- Red Team (Dokploy): 15 -->
- **Dọn image:** luôn giữ image đang chạy, image trong `PreviousSpec` của Swarm và 5 deployment **thành công** gần nhất; deployment lỗi tính riêng (giữ 3). Xoá manifest theo digest (`REGISTRY_STORAGE_DELETE_ENABLED`); GC cần registry read-only nên build/push của cụm bị hoãn trong lúc GC: chạy theo lịch hằng tuần trong cửa sổ bảo trì cấu hình được (mặc định Chủ nhật 03:00 theo timezone của cụm) và có nút chạy ngay; deployment trong lúc GC xếp hàng. <!-- Updated: Validation Session 1 - GC theo lịch + bấm tay --> <!-- Red Team: J -->
- **Kéo trước image:** sau khi deployment `healthy`, chạy global job kéo image hiện tại lên mọi node đủ điều kiện, để việc dời task khi build node chết không cần registry. <!-- Red Team: J -->
- **RBAC/audit/sự kiện:** route app, env, deploy, rollback, log, registry credentials có `x-permission` + audit; phát `deploy.succeeded|failed|rolled_back|cancelled` qua `events.Emit`. Phase 11 thêm cột `deployments.preview_id`; từ đó mọi quy tắc tuần tự, supersede, snapshot/rollback, giữ secret và image ở phase này áp theo khoá `(app_id, preview_id)` và production lọc `preview_id IS NULL`. <!-- Red Team (Dokploy): 1 --> <!-- Red Team: C -->
- **UI:** wizard tạo app, trang app (deployments, logs, env có cờ secret, domains, settings), Deploy và Rollback.

## Files to Create / Modify
- Create: `crates/cp/src/servicespec/` (app, job, infra, admission, secrets naming), `crates/cp/src/apps/`, `crates/cp/src/deploy/` (state machine, tuần tự hoá, snapshot), `crates/cp/src/registry/` (CA cụm, token server, dọn image)
- Create: `crates/agent/src/build/` (clone với GIT_ASKPASS, Dockerfile/Railpack, chạy `buildctl` trong container buildkit, quản lý container buildkitd + `buildkitd.toml`), `crates/agent/src/registryhost/` (container distribution, `/etc/hosts`, CA), `crates/agent/src/services/` (ensure service với version index, theo dõi rollout, logs)
- Modify: `crates/healthcheck/`, `crates/envexec/`, `crates/agent/src/nodeprep/`
- Create: `migrations/<timestamp>_apps_deployments.sql` (kèm `project_env`, `clusters.max_concurrent_builds`, `build_log_chunks`)
- Create: `crates/agent/src/cleanup/` (dọn Docker theo lịch cục bộ và theo ngưỡng disk), `crates/cp/src/envref/` (giải `${{project.KEY}}`)
- Modify: `proto/agent/v1/agent.proto`, `api/openapi.yaml`, `crates/cp/src/rbac/policy.rs`, `web/src/routes/apps/*`

## Tasks & Steps
1. `servicespec` + chính sách kiểm duyệt + network theo project (golden test).
2. CA cụm, registry container bind WG, `/etc/hosts`, token server.
3. buildkitd container + `buildkitd.toml`, `buildctl` exec + phân tích `rawjson`, GIT_ASKPASS + lọc log, Railpack, digest.
4. healthcheck + envexec; secret env.
5. Ensure service với version index; theo dõi rollout; xử lý `rolled_back`.
6. Deployment tuần tự + supersede + snapshot + rollback tay.
7. Log build/runtime; stream log build trực tiếp.
8. Dọn image có bảo vệ; kéo trước image.
9. Bộ điều phối build theo cụm; deployment không cần build bỏ qua slot; huỷ với `cancelling` + compare-and-set; timeout bước.
10. Dọn Docker trên node (danh sách giữ lại từ desired state, giới hạn cache BuildKit).
11. Biến tham chiếu `${{project.KEY}}`.
12. RBAC/audit/sự kiện; UI.

## Todo
- [ ] servicespec + admission + network project
- [ ] Registry: CA, container bind WG, tên ổn định, token server
- [ ] buildkitd + build + GIT_ASKPASS + Railpack + digest
- [ ] healthcheck + envexec + secret env
- [ ] Ensure service + xử lý rolled_back
- [ ] Tuần tự hoá + snapshot + rollback tay
- [ ] Log + stream log build trực tiếp
- [ ] Huỷ deployment (`cancelling` → `cancelled`, compare-and-set)
- [ ] Bộ điều phối build theo cụm + deployment không cần build bỏ qua slot + timeout bước
- [ ] Dọn Docker trên node + giới hạn cache BuildKit
- [ ] Biến `${{project.KEY}}`
- [ ] Dọn image có bảo vệ + kéo trước image
- [ ] RBAC/audit/sự kiện cho route mới
- [ ] UI app

## Success Criteria
- Deploy 3 app mẫu (Dockerfile bất kỳ, Node qua Railpack, `nginx` Docker Hub) đều có HTTPS.
- k6 trong lúc rolling update `replicas=3`: 0 request non-2xx.
- Healthcheck luôn lỗi: Swarm rollback, bản cũ vẫn phục vụ, deployment `rolled_back`, reconciler không áp lại bản hỏng (theo dõi 10 phút).
- Push A rồi B cách 20 giây: A thành `superseded` hoặc chạy xong trước B; không bao giờ A ghi đè B.
- Đổi env sai rồi Rollback: bản trước chạy với env cũ.
- Developer deploy app mount `/var/run/docker.sock` hoặc tự viết label `traefik.*`: bị từ chối.
- App project A không kết nối được tới service project B cùng cụm.
- Tắt build node: task bị dời vẫn khởi động được nhờ image đã kéo trước.
- Build trên build node 1 GB RAM không làm app khác trên node đó bị OOM-kill.
- Huỷ deployment đang `building`: trạng thái `cancelled`, không có image mới trong registry, bản đang chạy không đổi; nút huỷ không có ở deployment `rolling_out`.
- Huỷ khi agent build node đang mất kết nối: deployment ở `cancelling` vẫn giữ slot, deployment kế tiếp không bắt đầu; kết quả "đã push" tới trễ không đưa deployment đã huỷ sang `rolling_out`.
- Rollback tay của app B trong lúc app A đang build 15 phút: rollback của B chạy ngay, không chờ slot.
- Mở trang deployment khi build đang chạy: log hiện tiếp trong dưới 2 giây sau khi agent ghi; log không chứa token clone. Ngắt kết nối agent 2 phút giữa build: log lưu cuối cùng vẫn đầy đủ.
- `max_concurrent_builds=1`, đẩy 3 app cùng lúc: tại mọi thời điểm chỉ một deployment ở `cloning|building|pushing`; hai cái còn lại `queued` rồi lần lượt chạy.
- Lấp đĩa node tới 91%: dọn chạy ngay một lần, giải phóng image không dùng; image đã kéo trước, ops image, image của cron và mọi volume vẫn còn; disk vẫn trên 90% vì volume thì không dọn lại trước 30 phút và có cảnh báo `node.disk_high`; tắt control plane thì dọn theo lịch vẫn chạy.
- App tham chiếu `${{project.DB_HOST}}` và `${{project.API_KEY}}` (bí mật): biến thứ hai xuất hiện dưới dạng Swarm secret, không có trong `docker service inspect`; tham chiếu khoá không tồn tại làm deployment thất bại ở bước render.
- Đổi `project.DB_HOST` sang giá trị sai, redeploy rồi Rollback: bản trước chạy với giá trị `DB_HOST` cũ.

## Verification
- Golden test `servicespec` và admission; lab Incus: deploy, k6, healthcheck lỗi, push đồng thời, cách ly network, tắt build node.

## Risks
- Image có mount `noexec` hoặc entrypoint lạ làm `envexec`/healthcheck lỗi. Phản ứng: cho tắt healthcheck của platform và dùng HEALTHCHECK của image; cho phép secret ở dạng file thay vì env.
- Token server registry là phần tự viết theo spec Docker token auth. Tín hiệu: node không pull được. Phản ứng: test tích hợp với `distribution` thật trong CI.
- Registry chết thì không deploy được bản mới; task đang chạy và task dời có image kéo trước vẫn chạy. HA registry ngoài phạm vi v1.
