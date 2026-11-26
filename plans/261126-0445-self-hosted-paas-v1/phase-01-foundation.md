---
phase: 1
title: "Foundation"
status: pending
priority: P1
effort: "3w"
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
- **Auth:** user local (argon2id), session cookie HttpOnly + SameSite=Lax, CSRF token, rate limit đăng nhập. **Setup token:** install script sinh token dùng một lần, in ra terminal, lưu dạng hash; `/setup` đòi token này và vô hiệu sau khi tạo owner. <!-- Red Team: M -->
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
- `migrations/<timestamp>_init.sql` (users, sessions, setup_tokens, projects, memberships, audit_events, settings, encryption_keys)
- `api/openapi.yaml`, `web/`
- `deploy/controlplane/docker-compose.yml`, `deploy/install-controlplane.sh`
- `test/lab/incus-up.sh`, `test/lab/incus-down.sh`
- `.github/workflows/ci.yml`, `LICENSE`, `README.md`, `docs/development.md`

## Tasks & Steps
1. Module, Makefile (`make gen|test|lint|web|build`), golangci-lint, CI.
2. Migration init + sqlc + goose (timestamp) + River.
3. `secrets`, định danh control plane, setup token.
4. Auth (setup có token, login, session, CSRF, rate limit).
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

## Verification
- `make test`; `go test ./internal/cp/rbac/... -run TestAllRoutes`; chạy install script trên VM Incus.

## Risks
- Spec-first OpenAPI làm chậm vòng lặp đầu. Nếu cản trở quá một ngày, viết handler tay và sinh spec sau, nhưng giữ cổng `x-permission`.

## Security
- Master key và khoá Ed25519 của control plane chỉ đọc bởi user chạy control plane (0600). Mất master key là mất mọi secret: install script in cảnh báo và hướng dẫn backup.
