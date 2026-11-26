---
phase: 1
title: "Foundation"
status: pending
priority: P1
effort: "4.5w"
dependencies: []
---

# Phase 1: Foundation

## Goal
Có monorepo chạy được: control plane Go phục vụ SPA React qua TLS của chính nó, đăng nhập có bảo vệ màn hình setup, RBAC theo scope với cổng CI bắt mọi route khai báo quyền, audit cho cả thao tác thành công lẫn bị từ chối, contract phát sự kiện, job queue, lab test Incus, và CI xanh.

## Context
- Contract: `../reports/brainstorm-261126-1114-self-hosted-paas.md`, mục Key Decisions trong plan.md, research trong `research/`.
- Repo đang trống, chưa có `docs/`.
- Red Team (2026-11-26) đã chuyển vào phase này: cổng RBAC toàn route, audit lệnh bị từ chối, contract sự kiện, setup token, TLS tự phục vụ thay Caddy, migration đặt tên theo timestamp, lab Incus, chính sách URL đi ra (SSRF).

## Requirements
- Một Go module; các binary `cmd/controlplane`, `cmd/agent` (stub), `cmd/healthcheck` và `cmd/envexec` (stub, dùng ở phase 5).
- Control plane: chi router, cấu hình qua env + file, Postgres qua pgx v5, sqlc, goose với migration **đặt tên theo timestamp** (`YYYYMMDDHHMMSS_<name>.sql`, chỉ đi tiến) để các phase chạy song song không va số. River cho job queue. <!-- Red Team: N -->
- API spec-first: `api/openapi.yaml` → oapi-codegen (strict server) cho Go, openapi-typescript + openapi-fetch cho web. **Mọi operation phải có `x-permission: {action, scope}`.** <!-- Red Team: C -->
- **RBAC:** vai trò `owner | admin | developer | viewer`, scope `global | cluster | project`. Project là ranh giới cách ly bảo mật thật (người dùng chốt 2026-11-26): developer của project A không đọc/ghi được tài nguyên của project B. Ma trận quyền trong `internal/cp/rbac/policy.go`; middleware đọc `x-permission` của route. Test `TestAllRoutes` duyệt mọi operation × mọi vai trò và là **cổng CI**: route mới thiếu `x-permission` hoặc thiếu dòng trong ma trận làm CI đỏ. Các thao tác đọc nhạy cảm (log, connection string, xem secret, recovery kit) có action riêng, không gộp vào "read". <!-- Red Team: C -->
- **Audit:** `audit.Record(ctx, tx, ...)` cho thao tác ghi thành công (cùng transaction); `audit.RecordDenied(ctx, pool, ...)` được middleware RBAC gọi trong transaction riêng cho mọi request bị từ chối; đọc nhạy cảm cũng được ghi. <!-- Red Team: C -->
- **Sự kiện:** `events.Emit(ctx, tx, Event)` chèn một River job trong cùng transaction với thao tác. Mọi phase sau phát sự kiện qua hàm này; phase 8 chỉ thêm phần tiêu thụ (quy tắc + kênh thông báo). Không dựng bus riêng. <!-- Red Team: C -->
- **Auth:** user local (argon2id), session cookie HttpOnly + SameSite=Lax, tên có tiền tố `__Host-` (preview chạy mã chưa review có thể cùng site), CSRF token, rate limit đăng nhập. **Setup token:** install script sinh token dùng một lần, in ra terminal, lưu dạng hash; `/setup` đòi token này và vô hiệu sau khi tạo owner. <!-- Red Team: M -->
- **2FA TOTP** (RFC 6238): bật trong trang tài khoản bằng mã QR; secret TOTP mã hoá bằng master key (AAD = bảng + id user); 10 mã khôi phục dùng một lần lưu hash argon2id; từ chối dùng lại cùng bước thời gian đã dùng; rate limit chung với đăng nhập. Đăng nhập có 2FA là hai bước: mật khẩu đúng chỉ tạo phiên tạm (5 phút) để nhập mã. Owner có thể bật cài đặt "bắt buộc 2FA cho owner/admin": user thuộc vai trò đó chưa có 2FA bị đưa vào màn hình đăng ký 2FA trước khi làm việc khác. Chỉ owner reset được 2FA của người khác, cần xác thực lại và có audit. Lối thoát khi owner cuối cùng mất cả thiết bị lẫn mã khôi phục: lệnh `controlplane admin reset-2fa --user <email>` chạy trên host (cần shell của host và file master key), có audit; tài liệu ở phase 12. <!-- Red Team (Dokploy): 9 --> <!-- Updated: Dokploy comparison 2026-11-26 -->
- **Xác thực lại cho thao tác nhạy cảm:** `x-permission` có thêm trường `reauth: true`. Middleware chỉ cho qua khi phiên có `last_verified_at` trong 5 phút gần nhất; nếu không thì trả 403 mã `REAUTH_REQUIRED` (có `RecordDenied`) và UI hỏi lại mật khẩu, hoặc mã TOTP nếu user đã bật 2FA. Endpoint xác thực lại dùng chung rate limit đăng nhập. Danh sách route `reauth` là dữ liệu trong `internal/cp/rbac/policy.go`; `TestAllRoutes` đọc danh sách đó, và phase nào tạo route nhạy cảm thì đánh dấu `reauth: true` ngay trong phase đó (phase 2, 5, 7, 9). <!-- Red Team (Dokploy): 9 --> Danh sách ban đầu: tải recovery kit, xem giá trị secret, bật `privileged`, mở public database, xoá cụm/project, tạo API token, tắt hoặc reset 2FA. Request dùng API token không bao giờ thoả `reauth`, nên các thao tác này chỉ làm được trên UI. `TestAllRoutes` kiểm rằng mọi route trong danh sách có `reauth: true`. <!-- Updated: Dokploy comparison 2026-11-26 -->
- **API token cho CI:** token cá nhân gắn với user, định dạng `bnp_<ngẫu nhiên 32 byte base62>` (tiền tố giúp secret scanning nhận diện), chỉ hiện một lần lúc tạo, lưu SHA-256. Mỗi token có danh sách project (tập con của membership của user), vai trò không cao hơn vai trò của user trong project đó, hạn dùng bắt buộc (tối đa 1 năm), `last_used_at`, thu hồi được. Gửi qua `Authorization: Bearer`; đi qua cùng middleware RBAC và `x-permission`; audit ghi cả user lẫn id token. Vai trò hiệu lực được tính **ở mỗi request** = thấp hơn giữa vai trò lưu trong token và vai trò hiện tại của user trong project; token chỉ dùng được cho route scope `project` (route scope `cluster`/`global` luôn 403, `TestAllRoutes` có cột cho principal token); token của user thuộc diện bắt buộc 2FA mà chưa bật 2FA bị từ chối. <!-- Red Team (Dokploy): 9 --> Gỡ user khỏi project hoặc khỏi bản cài (phase 10) thu hồi ngay token liên quan. Không làm URL deploy chứa token như Dokploy. <!-- Updated: Dokploy comparison 2026-11-26 -->
- **Mã hoá secret at-rest:** AES-256-GCM, AAD = tên bảng + id bản ghi, master key 32 byte đọc từ file 0600, cột `key_id` cho việc xoay khoá.
- **Định danh control plane:** lúc cài sinh `cp_instance_id`, cặp khoá Ed25519 của control plane (dùng ở phase 2) và `cp_epoch` (UUID, đổi mỗi lần restore từ backup — phase 2 dùng).
- **TLS:** control plane tự nghe 80/443 và xin cert cho domain của nó bằng `internal/cp/certs` (lego v5, HTTP-01) — cùng package phase 4 mở rộng cho domain của app. Không dùng Caddy. Tuỳ chọn chạy sau proxy ngoài (tắt TLS, khai báo trusted proxy). <!-- Red Team: N -->
- **Chính sách URL đi ra** (`internal/cp/netpolicy`): mọi URL do người dùng nhập (base URL Git, webhook thông báo, endpoint S3 custom, git URL) chỉ cho `https`, chặn link-local/RFC1918/dải WireGuard/loopback trừ khi admin cho phép, không theo redirect, không trả nội dung response trong thông báo lỗi. <!-- Red Team: M -->
- Web: React + Vite + TypeScript, TanStack Router + TanStack Query, Tailwind + shadcn/ui; build nhúng bằng `go:embed`.
- Cài control plane: `deploy/controlplane/docker-compose.yml` (controlplane + Postgres); `deploy/install-controlplane.sh` sinh master key, khoá Ed25519, setup token, `.env`, rồi `docker compose up -d`.
- **Lab test:** `test/lab/incus-up.sh` dựng 3 VM Incus (Ubuntu LTS, Docker) để các phase 2–10 kiểm trên kernel thật. <!-- Red Team: N -->
- Giấy phép AGPL-3.0 (`LICENSE`, header SPDX).

## Files to Create
- `go.mod`, `Makefile`, `.golangci.yml`, `sqlc.yaml`, `buf.yaml`
- `cmd/controlplane/main.go`, `cmd/agent/main.go`, `cmd/healthcheck/main.go`, `cmd/envexec/main.go`
- `internal/cp/{config,httpserver,auth,rbac,audit,events,secrets,store,jobs,certs,netpolicy,webui}/`
- `migrations/<timestamp>_init.sql` (users, sessions, setup_tokens, projects, memberships, audit_events, settings, encryption_keys, user_totp, recovery_codes, api_tokens)
- `api/openapi.yaml`, `web/`
- `deploy/controlplane/docker-compose.yml`, `deploy/install-controlplane.sh`
- `test/lab/incus-up.sh`, `test/lab/incus-down.sh`
- `.github/workflows/ci.yml`, `LICENSE`, `docs/development.md`
- Modify: `README.md` (add build and run instructions), `docs/system-architecture.md` (link boundaries to real packages)

## Tasks & Steps
1. Module, Makefile (`make gen|test|lint|web|build`), golangci-lint, CI.
2. Migration init + sqlc + goose (timestamp) + River.
3. `secrets`, định danh control plane, setup token.
4. Auth (setup có token, login, session, CSRF, rate limit); 2FA TOTP + mã khôi phục + cài đặt bắt buộc; xác thực lại (`reauth`); API token.
5. RBAC: `x-permission`, policy, middleware, `TestAllRoutes` làm cổng CI.
6. Audit: `Record` + `RecordDenied` + test.
7. `events.Emit` + test (rollback transaction thì không có job).
8. `certs` tối thiểu (HTTP-01 cho domain control plane), server TLS.
9. `netpolicy` + test chặn dải cấm.
10. SPA shell; compose + install script; lab Incus.

## Todo
- [ ] Module, Makefile, lint, CI
- [ ] Migration timestamp, sqlc, goose, River
- [ ] Secrets, định danh control plane, setup token
- [ ] Auth
- [ ] 2FA TOTP + mã khôi phục + bắt buộc cho owner/admin
- [ ] Xác thực lại 5 phút cho route `reauth: true`
- [ ] API token cho CI (scope project, hạn dùng, thu hồi)
- [ ] RBAC + `x-permission` + cổng `TestAllRoutes`
- [ ] Audit thành công + bị từ chối + đọc nhạy cảm
- [ ] `events.Emit`
- [ ] Cert tự phục vụ cho control plane
- [ ] `netpolicy`
- [ ] SPA, compose, install script, lab Incus

## Success Criteria
- `make lint test build` xanh; thêm một route không có `x-permission` thì CI đỏ.
- Install script trên VM sạch đưa control plane lên HTTPS dưới 5 phút; `/setup` không có token trả 403.
- Viewer gọi endpoint ghi nhận 403 và `audit_events` có một bản ghi `denied`.
- `netpolicy` từ chối `http://169.254.169.254`, `https://10.77.0.1`, `https://127.0.0.1`.
- `test/lab/incus-up.sh` dựng 3 VM trong dưới 10 phút.
- User có 2FA: đăng nhập chỉ bằng mật khẩu không vào được; dùng lại mã TOTP vừa dùng bị từ chối; mỗi mã khôi phục chỉ dùng được một lần.
- Tạo API token khi phiên đã xác thực hơn 5 phút trước trả `REAUTH_REQUIRED`; sau khi nhập lại mật khẩu thì tạo được; mọi route `reauth: true` gọi bằng API token đều bị 403. (Kiểm recovery kit nằm ở phase 7.) <!-- Red Team (Dokploy): 9 -->
- API token scope project A gọi API của project B nhận 403; token hết hạn hoặc bị thu hồi nhận 401; `curl` dùng token gọi deploy được ghi audit với id token.
- Token tạo khi user là admin của project A, sau đó user bị hạ xuống viewer: token bị 403 khi deploy. Token gọi route scope `cluster` bị 403.

## Verification
- `make test`; `go test ./internal/cp/rbac/... -run TestAllRoutes`; chạy install script trên VM Incus.

## Risks
- Spec-first OpenAPI làm chậm vòng lặp đầu. Nếu cản trở quá một ngày, viết handler tay và sinh spec sau, nhưng giữ cổng `x-permission`.

## Security
- Master key và khoá Ed25519 của control plane chỉ đọc bởi user chạy control plane (0600). Mất master key là mất mọi secret: install script in cảnh báo và hướng dẫn backup.
