---
phase: 11
title: "Preview Deployments"
status: pending
priority: P2
effort: "3w"
dependencies: [4, 5, 6, 7, 8, 10]
---

# Phase 11: Preview Deployments

## Goal

Mỗi pull request (GitHub, GHES, GitLab merge request, Gitea/Forgejo) của một app có bật preview được build và chạy thành một bản preview có URL HTTPS riêng, chỉ khi cả tác giả PR lẫn người đẩy commit đều có quyền ghi vào repo; bản preview không bao giờ nhận secret, network, cache build hay lịch sử deployment của production, có giới hạn số lượng, và bị gỡ khi PR đóng hoặc merge.

## Context

- Nguồn ý tưởng: preview deployment của Dokploy (`services/preview-deployment.ts`, `pages/api/deploy/github.ts` tại commit `6b02c8d5`, Apache-2.0); chỉ lấy hành vi, không chép mã. So sánh đầy đủ ở `../reports/xia-261126-1619-dokploy-comparison.md`. Người dùng đưa tính năng này vào v1 ngày 2026-11-26.
- Khác Dokploy có chủ đích: Dokploy dùng wildcard `sslip.io` và cho preview nhận env của app; Bennghe dùng zone trong allowlist của project, cert wildcard cấp một lần cho mỗi cụm, và tách preview khỏi production ở cả lúc build lẫn lúc chạy.
- Phạm vi: chỉ app (như Dokploy). Stack compose và bản sao database cho preview không nằm trong phase này (ghi vào known limitations ở phase 12).
- Red-team ngày 2026-11-26 (phần bổ sung từ Dokploy) đã viết lại phase này; marker `Red Team (Dokploy): n` trỏ về bảng trong plan.md.

## Requirements

### Cấu hình

- `app_preview_settings(app_id, enabled, limit DEFAULT 3)`; env riêng `app_preview_env(key, value_enc, is_secret)`. Cấu hình tài nguyên preview là hằng số của platform: 1 replica, 0.5 CPU, 512 MB. Chỉ người có quyền quản lý app mới bật/tắt và sửa env preview được. <!-- Red Team (Dokploy): 15 -->
- **Preview zone theo `(project, cụm)`** `project_preview_zones(project_id, cluster_id, suffix, exposure: direct|tunnel)`, admin cấu hình. Bật preview cho một app cần zone của project trên cụm của app đó. <!-- Red Team (Dokploy): 4 -->
  - `suffix` phải nằm trong allowlist zone của project (phase 4). Nếu suffix nằm dưới **registrable domain** của control plane hoặc của domain production trong project (tính bằng public suffix list `golang.org/x/net/publicsuffix`) thì UI hiện cảnh báo rõ rủi ro cùng site và khuyến nghị dùng một registrable domain riêng cho preview (ví dụ `example-preview.dev`), nhưng vẫn cho lưu; cookie của control plane đã có tiền tố `__Host-` (phase 1). <!-- Updated: Validation Session 2 - chỉ cảnh báo, không chặn zone cùng registrable domain --> <!-- Red Team (Dokploy): 13 -->
  - **`exposure=direct`:** control plane cấp **một** cert wildcard `*.<suffix>` qua DNS-01 Cloudflare, chỉ phân phối tới Traefik của cụm đó, và tạo **một** record wildcard A `proxied=false` (mang dấu `managed-by: bennghe`) trỏ tới IP public của các node ingress của cụm đó. Preview mới không xin cert và không tạo record riêng.
  - **`exposure=tunnel`:** không tạo record A (không lộ IP gốc). Cloudflare terminate TLS ở edge, mà Universal SSL chỉ phủ apex và **một** cấp subdomain, nên `suffix` phải đúng bằng apex của một zone Cloudflare: hostname preview nằm ngay dưới apex. Mỗi preview có một CNAME proxied tới tunnel và một dòng trong cấu hình ingress của tunnel (cơ chế dựng lại toàn bộ danh sách của phase 10).
- **Hostname:** `pr-<số PR>-<slug app, cắt>-<4 ký tự hex từ id app>.<suffix>`, nhãn đầu không quá 63 ký tự; phần hex tránh va chạm `hostname UNIQUE` giữa hai app có slug cắt giống nhau.
- **Dòng `domains` của preview:** `owner_type=preview`, `tls_mode=preview_zone` (giá trị enum mới: dùng cert wildcard của zone, cert manager không xin cert riêng), `exposure` sao chép từ zone. Module `domains`/`cloudflare` bỏ qua bước tạo record A theo từng domain cho `owner_type=preview`; với tunnel thì tạo CNAME riêng như trên. Dòng do actor hệ thống (worker webhook) tạo, có audit với actor = kết nối provider. <!-- Red Team (Dokploy): 4 -->

### Sự kiện PR và cổng chấp nhận

- **Interface `Provider`** (phase 6) thêm ba phương thức, mỗi provider một bản thật:
  ```go
  ParsePullRequestEvent(r *http.Request, secret []byte) (PREvent, error) // opened|synchronize|reopened|closed; mang head_sha, head repo, author, sender
  CanWrite(ctx, conn, repo, username string) (bool, error)
  GetPullRequest(ctx, conn, repo string, number int) (PRState, error)   // dùng cho quét dọn và kiểm head trước khi clone
  ```
  - GitHub/GHES: sự kiện `pull_request`; quyền qua `GET /repos/{owner}/{repo}/collaborators/{username}/permission`, hợp lệ khi trường `permission` là `admin` hoặc `write` (API ánh xạ `maintain` thành `write`) [UNVERIFIED: quyền App tối thiểu cho endpoint này; kiểm trên org thử].
  - GitLab: merge request hook; quyền qua `GET /projects/:id/members/all/:user_id` với access level ≥ Developer (30); người kích hoạt lấy từ trường `user` của hook.
  - Gitea/Forgejo: sự kiện `pull_request`; quyền qua API permission của collaborator (endpoint `GET /repos/{owner}/{repo}/collaborators/{collaborator}/permission` có trong tài liệu API Gitea 1.22; bản hỗ trợ thấp nhất vẫn kiểm bằng container CI; Forgejo best-effort).
  - Người kích hoạt sự kiện: GitHub và Gitea là `sender`, GitLab là `user` ("User who triggered the event" theo tài liệu webhook GitLab; Gitea ghi `sender` là "the user who performed the action"). Contract test có hai tài khoản vẫn kiểm rằng với sự kiện cập nhật PR đây chính là người đẩy commit. <!-- Red Team (Dokploy): 2 --> <!-- Updated: Validation Session 2 - xác minh trường người kích hoạt -->
- **Webhook:** phase 6 đã đăng ký sự kiện PR cho mọi hook và trả 2xx bỏ qua khi chưa có phase này; phase 11 chỉ thêm bộ xử lý. Khoá khử trùng của sự kiện PR là `(connection, repo, pr_number, action, head_sha)`; khoá của push giữ nguyên. Bật preview cho app gọi lại `EnsureWebhook` (idempotent) để chắc hook có sự kiện PR. <!-- Red Team (Dokploy): 5 -->
- **Cổng chấp nhận** cho `opened|synchronize|reopened` (kiểm theo thứ tự; bị từ chối thì ghi lý do vào lịch sử preview và đặt commit status context `bennghe/preview` về `failure` kèm lý do):
  1. Chữ ký webhook, khử trùng, và hỏi lại provider rằng `head_sha` vẫn là đầu PR.
  2. App bật preview; PR nhắm vào branch mà app đang theo dõi.
  3. **Cả tác giả PR lẫn `sender` của sự kiện** (người mở PR hoặc người đẩy commit) đều có quyền ghi (`CanWrite`), kiểm trực tiếp ở mỗi sự kiện, không cache. PR từ fork chỉ được build khi cả hai điều kiện này đúng; người có quyền đẩy vào fork nhưng không có quyền ghi repo gốc sẽ bị chặn ở chính sự kiện `synchronize` họ tạo ra. <!-- Red Team (Dokploy): 2 -->
  4. Số preview đang hoạt động của app nhỏ hơn `limit`, kiểm dưới `SELECT … FOR UPDATE` trên dòng `app_preview_settings`; đủ rồi thì bỏ qua và báo qua commit status, không đẩy preview cũ ra. <!-- Red Team (Dokploy): 11 -->
- **Ghim SHA:** deployment preview lưu `source_sha` = `head_sha` đã qua cổng. Ngay trước khi chuyển sang `cloning`, bộ điều phối của phase 5 gọi lại `GetPullRequest`; nếu head đã đổi thì deployment thành `superseded` (sự kiện mới tự đi qua cổng). Agent fetch `refs/pull/<n>/head` (GitHub, Gitea) hoặc `refs/merge-requests/<iid>/head` (GitLab) từ repo gốc bằng credential ngắn hạn của phase 5–6, rồi `git checkout --detach <source_sha>`; không có SHA đó thì thất bại. <!-- Red Team (Dokploy): 2 -->

### Cách ly khỏi production

- **Lịch sử deployment:** `deployments.preview_id` (NULL = production, FK tới `previews`). Mọi quy tắc của phase 5 và 6 áp theo khoá `(app_id, preview_id)`: unique job/advisory lock, supersede, snapshot thành công trước đó cho rollback tay và Swarm tự rollback, giữ secret của 5 deployment thành công, chặn tự deploy commit cũ hơn, base của watch paths, giữ image. Production luôn lọc `preview_id IS NULL`, nên rollback của production không bao giờ chọn snapshot của preview. <!-- Red Team (Dokploy): 1 -->
- **Build:** build preview chạy trên một **buildkitd riêng** (`bn-buildkitd-preview`, container thường trên build node, thư mục state riêng, cùng giới hạn RAM/CPU của phase 5; agent chỉ khởi động nó khi có build preview và dừng sau 10 phút rảnh, state giữ trên đĩa <!-- Updated: Validation Session 2 - buildkitd preview bật khi cần -->) với `cache-key` riêng cho preview, để PR không ghi được vào cache mount hay cache layer mà build production dùng lại. Token server của registry chỉ cấp cho buildkitd này quyền push vào repo `preview/<project>/<app>`. Secret lúc build (`--secret`) của preview chỉ lấy từ `app_preview_env`. <!-- Red Team (Dokploy): 3 -->
- **Env lúc chạy:** env preview = biến thường của app có giá trị là chữ (literal) + `app_preview_env`. Khoá khai báo trong `app_preview_env` **thay hẳn** biến cùng tên của app. Bộ giải `${{project.KEY}}` (phase 5) chạy ở chế độ preview: không bao giờ giải biến bí mật của project; tham chiếu tới biến bí mật làm render thất bại với thông báo rõ. Việc lọc dựa trên **độ bí mật sau khi giải**, không dựa trên cờ `is_secret` của dòng app. Env bí mật của app và `DATABASE_URL` từ liên kết database (phase 7) không bao giờ được kế thừa. <!-- Red Team (Dokploy): 7 -->
- **Secret và network:** secret preview là Swarm secret tiền tố `bn-prev-`; không gắn preview vào `bn-db-<id>`. Preview chạy trong overlay `bn-projprev-<project>` của cụm (không attachable) với subnet cấp từ dải dành riêng cho preview của cụm (mặc định `10.240.0.0/12`); Traefik được gắn vào network này **một lần** cho mỗi project trong cụm (cùng đánh đổi rolling restart đã chấp nhận ở phase 4). <!-- Red Team (Dokploy): 13 -->
- **Không đi vòng qua Traefik:** agent ingress thêm `&& !ClientIP(\`<dải preview>\`)` vào rule của mọi router không phải preview, nên container preview gửi request tới Traefik với header `Host` của app production (kể cả entrypoint HTTP nội bộ của tunnel) chỉ nhận 404. Cú pháp `ClientIP` và phủ định `!` có trong tài liệu rule của Traefik v3 (khớp theo IP kết nối, không theo `X-Forwarded-For`) [UNVERIFIED: Traefik thấy IP overlay của task preview làm client IP khi đi qua VIP của Swarm; kiểm trong lab]. <!-- Red Team (Dokploy): 13 -->
- Chính sách kiểm duyệt workload của phase 5 áp nguyên vẹn; cờ `privileged` của app không được chuyển sang preview.

### Thực thi và vòng đời

- Mỗi preview là một Swarm service `bn-prev-<app-id>-<pr>` sinh qua `servicespec`. Deployment preview đi qua state machine, snapshot và bộ điều phối build của phase 5 với khoá `(app_id, preview_id)`. Bộ điều phối xếp production trước preview; preview đã chờ quá 30 phút được xếp ngang production để không chờ mãi. Commit mới trên PR làm build preview cũ chưa tới `rolling_out` thành `superseded`. <!-- Red Team (Dokploy): 11 -->
- **Gỡ preview là một mục trong cùng hàng đợi `(app_id, preview_id)`:** trước hết chuyển mọi deployment chưa kết thúc của preview sang `cancelling` (phase 5), rồi xoá service, secret `bn-prev-*`, dòng `domains` (agent ingress bỏ route; với tunnel thì xoá CNAME và dựng lại cấu hình tunnel), đặt preview thành `removed` (tombstone). Kết quả đến sau cho một preview `removed` bị bỏ và agent được yêu cầu xoá service nếu nó còn. Image preview được đánh dấu để dọn và không tính vào quota giữ image của production. <!-- Red Team (Dokploy): 6 -->
- **Khi nào gỡ:** PR `closed` (kể cả merge); người dùng tắt preview của app; xoá app; đổi repo hoặc branch của app; xoá kết nối provider. <!-- Red Team (Dokploy): 6, 15 -->
- **Quét dọn mỗi giờ** (River periodic job): với mỗi preview đang hoạt động, gọi `GetPullRequest`: PR đã đóng → gỡ; 404/401/403 (repo chuyển, installation bị thu hồi) → gỡ và ghi lý do; lỗi 5xx hoặc timeout → bỏ qua lượt này. Đồng thời gỡ mọi service `bn-prev-*` trong cụm không ứng với preview đang hoạt động. <!-- Red Team (Dokploy): 6 -->
- **Dọn tài nguyên cấp zone:** khi cụm không còn app nào của project bật preview, hoặc project bị xoá, hoặc admin xoá zone: tách Traefik khỏi `bn-projprev-<project>`, xoá network, xoá record wildcard, xoá cert wildcard (ngừng gia hạn). <!-- Red Team (Dokploy): 6 -->
- **Commit status:** context riêng `bennghe/preview`: `pending` khi bắt đầu, `success` với target URL là URL preview, `failure` khi lỗi hoặc bị cổng từ chối, `error` "đã huỷ" khi bị huỷ; không đụng tới status của deployment production.
- **RBAC/audit/sự kiện:** route cấu hình preview, preview zone (admin), env preview, xoá preview tay, xem log preview có `x-permission` + `audit.Record`; xem log preview dùng action đọc nhạy cảm như log app. Phát `preview.deployed|failed|removed` qua `events.Emit`; phase 8 đã có kênh thông báo nên chỉ cần thêm loại sự kiện vào quy tắc. <!-- Red Team (Dokploy): 15 -->
- **UI:** tab Preview trong trang app (bật/tắt, limit, env preview, danh sách preview với PR, tác giả, trạng thái, URL, nút xoá); cấu hình preview zone theo cụm trong trang project (admin).

## Files to Create / Modify

- Create: `internal/cp/previews/` (cổng chấp nhận, vòng đời, gỡ, quét dọn, dọn zone), `migrations/<timestamp>_previews.sql` (`app_preview_settings`, `app_preview_env`, `previews`, `project_preview_zones`, `deployments.preview_id`, enum `domains.owner_type=preview`, enum `tls_mode=preview_zone`)
- Modify: `internal/cp/gitprovider/provider.go`, `internal/cp/gitprovider/{github,gitlab,gitea}/` (sự kiện PR, `CanWrite`, `GetPullRequest`), `internal/cp/webhooks/` (bộ xử lý sự kiện PR, khoá khử trùng PR), `internal/cp/deploy/` (khoá `(app_id, preview_id)`, lọc production, ưu tiên và aging trong bộ điều phối, kiểm head trước khi clone), `internal/cp/envref/` (chế độ preview), `internal/cp/servicespec/` (spec preview, network `bn-projprev-*` với subnet dành riêng), `internal/cp/domains/` (owner `preview`, bỏ qua record A), `internal/cp/routing/` (rule `!ClientIP` cho router production), `internal/cp/certs/` (cert wildcard theo zone, `preview_zone`), `internal/cp/cloudflare/` (record wildcard, CNAME tunnel), `internal/cp/tunnel/` (hostname preview), `internal/cp/registry/` (repo `preview/*`, quyền push riêng, dọn image preview)
- Modify: `internal/agent/build/` (buildkitd preview riêng, fetch ref PR + checkout SHA), `internal/agent/ingress/`, `internal/agent/cleanup/` (giữ image preview đang chạy)
- Modify: `proto/agent/v1/agent.proto`, `api/openapi.yaml`, `internal/cp/rbac/policy.go`, `web/src/routes/apps/*/previews*`, `web/src/routes/projects/*/settings*`

## Tasks & Steps

1. Migration; `deployments.preview_id` và lọc production trong mọi truy vấn của `internal/cp/deploy/` (test hồi quy rollback).
2. Preview zone theo (project, cụm): direct (cert + record wildcard) và tunnel (suffix = apex, CNAME riêng); chặn registrable domain của control plane.
3. Mở rộng `Provider` (sự kiện PR, `CanWrite`, `GetPullRequest`) cho GitHub/GHES, GitLab, Gitea; contract test bằng fixture có hai tài khoản (tác giả và người đẩy).
4. Bộ xử lý webhook PR, khoá khử trùng PR, cổng chấp nhận (tác giả + sender, limit có khoá dòng), ghim SHA.
5. Build cách ly: buildkitd preview, cache-key riêng, repo `preview/*`, secret build từ env preview.
6. Spec preview: env chế độ preview, secret `bn-prev-*`, network với subnet dành riêng, rule `!ClientIP` cho router production.
7. Nối vào bộ điều phối của phase 5 (ưu tiên, aging, supersede, kiểm head trước clone).
8. Gỡ preview qua hàng đợi + tombstone; các điểm kích hoạt gỡ; quét dọn; dọn tài nguyên zone.
9. Commit status `bennghe/preview`; sự kiện; RBAC/audit.
10. UI tab Preview và cấu hình preview zone.

## Todo

- [ ] `deployments.preview_id` + lọc production ở mọi quy tắc phase 5/6
- [ ] Preview zone theo (project, cụm): direct và tunnel, chặn registrable domain của control plane
- [ ] `Provider`: sự kiện PR, `CanWrite`, `GetPullRequest` (3 provider)
- [ ] Webhook PR, khoá khử trùng PR, cổng chấp nhận tác giả + sender, ghim SHA
- [ ] Build cách ly (buildkitd preview, cache, repo, secret build)
- [ ] Spec preview cách ly (env, secret, network, `!ClientIP`)
- [ ] Nối bộ điều phối (ưu tiên, aging, supersede, kiểm head)
- [ ] Gỡ preview qua hàng đợi, quét dọn, dọn zone
- [ ] Commit status, sự kiện, RBAC/audit cho route mới
- [ ] UI

## Success Criteria

- Mở PR từ branch của collaborator trên mỗi provider (GitHub.com, GHES, GitLab, Gitea; Forgejo best-effort): preview HTTPS chạy tại hostname preview và commit status `bennghe/preview` chuyển `success` với URL preview; làm được trên cả một cụm `direct` và một cụm `tunnel`.
- Đẩy commit mới lên PR: preview cập nhật về SHA mới; build cũ chưa `rolling_out` bị `superseded`.
- PR từ fork của người không có quyền ghi: không build. PR của collaborator nhưng commit mới do một tài khoản không có quyền ghi repo gốc đẩy vào fork: commit đó không được build và commit status báo bị từ chối.
- Đẩy commit Y ngay sau khi commit X qua cổng: deployment của X không bao giờ build mã của Y.
- Preview `healthy`, sau đó deployment production lỗi và Swarm tự rollback: production quay về snapshot production trước đó, không phải snapshot preview.
- Một PR ghi dữ liệu vào cache mount (`--mount=type=cache`) và cache layer: build production kế tiếp cho ra cùng digest như khi không có PR đó; bước build của preview không đọc được secret build nào của production.
- Từ bên trong container preview: không kết nối được tới service production hay database đã liên kết; gửi request tới Traefik với header `Host` của app production nhận 404; `docker service inspect` của preview không tham chiếu secret nào của app production; app production có env `${{project.API_KEY}}` (bí mật) và env preview khai báo `API_KEY` thì preview nhận giá trị trong env preview, không phải giá trị của project.
- Đạt `limit` (3), mở đồng thời hai PR mới: không bao giờ có quá 3 preview hoạt động.
- Đóng PR khi build preview đang chạy: build bị huỷ, không có service `bn-prev-*` nào còn lại sau 1 phút. Tắt control plane lúc đóng PR rồi bật lại: quét dọn gỡ preview trong vòng 1 giờ.
- Tạo 5 preview liên tiếp trên cụm `direct` không phát sinh yêu cầu ACME mới nào sau cert wildcard đầu tiên; tắt preview của app cuối cùng trong cụm thì record wildcard, cert wildcard và network `bn-projprev-*` bị xoá.
- Cấu hình preview zone dưới registrable domain của control plane: lưu được nhưng UI hiện cảnh báo rủi ro cùng site.
- Không có build preview trong 10 phút: container `bn-buildkitd-preview` dừng; build preview kế tiếp tự khởi động lại nó.

## Verification

- Contract test bằng fixture cho sự kiện PR, `CanWrite` (tác giả và sender khác nhau) và `GetPullRequest` của cả ba provider trong CI.
- Test sống: Gitea container trong CI (PR từ branch, từ fork của người không có quyền ghi, và commit do tài khoản ngoài đẩy vào fork của collaborator); GitHub org thử và GitLab CE trong job nightly; GHES trial chạy tay trước khi đóng phase.
- Lab Incus: cách ly network, rule `!ClientIP`, secret, cache build, rollback production sau preview; một zone Cloudflare thử cho chế độ tunnel.

## Risks

- **Người có quyền ghi bị chiếm tài khoản hoặc cố ý đẩy mã độc** vẫn chạy được mã trên buildkitd preview và trong `bn-projprev-*`. Phản ứng: buildkitd riêng, không có secret production, chính sách kiểm duyệt; ghi rõ trong `docs/security-model.md` (phase 12) rằng bật preview nghĩa là tin mọi người có quyền ghi vào repo.
- **Preview ngốn tài nguyên VPS nhỏ.** Phản ứng: giới hạn mặc định 3 preview, hồ sơ 0.5 CPU / 512 MB, build preview xếp sau production.
- **Quyền App GitHub cho endpoint permission chưa xác nhận.** Tín hiệu: contract test với org thử trả 403. Phản ứng: thêm quyền tối thiểu cần thiết vào manifest của phase 6 trước khi phát hành alpha có GitHub App.
- **`ClientIP` không thấy IP overlay của task preview** (ví dụ do NAT của VIP). Tín hiệu: test trong lab vẫn tới được app production qua Traefik. Phản ứng: chạy một Traefik riêng cho preview chỉ gắn vào các network `bn-projprev-*` và chỉ phục vụ hostname preview; hỏi người dùng vì cần thêm port.
- **Zone tunnel bắt suffix là apex** làm hostname preview nằm ngay dưới domain chính. Đây là hệ quả của giới hạn Universal SSL; người dùng có Advanced Certificate Manager có thể muốn nới, nhưng v1 không làm.
- **Traefik rolling restart khi project bật preview lần đầu trong cụm** (gắn `bn-projprev-*`). Cùng đánh đổi người dùng đã chấp nhận ở Validation Session 1.
