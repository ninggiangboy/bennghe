---
phase: 7
title: "Databases & Backups"
status: pending
priority: P1
effort: "3.5w"
dependencies: [5]
---

# Phase 7: Databases & Backups

## Goal
Tạo Postgres, MySQL, MongoDB, Redis bằng một thao tác trong network riêng; backup lên S3-compatible (AWS, R2, B2, MinIO) có mã hoá, retention, credential đặt trong cụm an toàn để backup chạy được khi control plane offline; restore vào database mới. Lịch backup chạy qua scheduler ở phase 8.

## Requirements
- **Database service** (qua `servicespec`): image chính thức theo phiên bản chọn; ghim bằng `node.labels.bn.node==<id>` + volume local; `update_config.order=stop-first`; **network riêng `bn-db-<id>`** chỉ gắn database và các app được liên kết; mật khẩu sinh ngẫu nhiên, lưu mã hoá, đưa vào bằng Swarm secret (`*_PASSWORD_FILE`). <!-- Red Team: E -->
- **Liên kết với app:** gắn app vào `bn-db-<id>` và thêm `DATABASE_URL` dạng **secret env** (phase 5), rồi redeploy (đi qua hàng đợi tuần tự của app).
- **Mở public** (đã được người dùng xác nhận là trong phạm vi): chỉ admin, route có `reauth: true` (phase 1) <!-- Red Team (Dokploy): 9 -->; publish port mode `host` trên node của database, quy tắc `DOCKER-USER` chỉ cho IP trong allowlist; UI cảnh báo rủi ro; có audit.
- UI nói rõ: database không có HA trong v1; khôi phục bằng restore.
- **Ops image** `images/ops/Dockerfile`: pg_dump/psql (client mới nhất), mysqldump/mysql, mongodump/mongorestore, redis-cli, restic. Phát hành lên GHCR ở phase 12; dev build vào registry của cụm.
- **Đích backup:** `backup_destinations(name, preset: aws|r2|b2|minio|custom, endpoint, region, bucket, prefix, backup_key_enc, backup_secret_enc, prune_key_enc, prune_secret_enc)`. Preset R2: `https://<account-id>.r2.cloudflarestorage.com`, region `auto`. Endpoint `custom` đi qua `netpolicy`. UI khuyến nghị: credential backup chỉ có quyền ghi (không xoá) theo prefix; credential prune riêng có quyền xoá; R2 không có object lock nên khuyến nghị bật versioning/lifecycle nếu nhà cung cấp hỗ trợ. <!-- Red Team: B -->
- **Credential trong cụm:** <!-- Red Team: B -->
  - Control plane tạo Swarm secret `bn-backup-<db-id>-<hash>` chứa mật khẩu restic + credential **backup** (không có credential prune), và Swarm secret mật khẩu database. Lịch và spec job chỉ tham chiếu **tên** secret. Không đặt credential trong env hay Swarm config.
  - Đổi credential: tạo secret mới, cập nhật tham chiếu, xoá secret cũ khi không còn job nào dùng.
- **Chạy backup:** service `replicated-job` qua `servicespec` (đặt tường minh `MaxConcurrent` và `TotalCompletions`), ghim vào node của database, gắn `bn-db-<id>`, mount secret, chạy `restic backup --stdin-from-command --stdin-filename <db>.dump -- <lệnh dump>` vào repo `s3:<endpoint>/<bucket>/<prefix>/<db-id>`. Lệnh dump lỗi thì restic huỷ snapshot.
- **Retention/prune:** `restic forget --keep-daily N --keep-weekly M --keep-monthly K --prune` chạy như job **riêng**, do control plane điều phối (cần control plane online; không gấp về thời gian), mount credential prune trong một Swarm secret tạo ngay trước job và xoá ngay sau job. Backup và prune trên cùng repo không bao giờ chạy đồng thời (khoá theo repo ở control plane; scheduler bỏ qua lượt backup nếu prune đang giữ khoá và ghi lý do). <!-- Red Team: B -->
- **Recovery kit:** chỉ owner/admin tải được, route có `reauth: true` (phase 1) <!-- Red Team (Dokploy): 9 -->; mỗi lần tải được audit; chứa endpoint, bucket, mật khẩu restic. <!-- Red Team: B -->
- **Restore:** tạo database mới cùng engine/phiên bản, chạy job `restic dump` đổ vào client tương ứng; không ghi đè database đang chạy.
- **Backup volume** của app: `restic backup` thư mục volume mount read-only; UI ghi rõ không đảm bảo nhất quán khi đang ghi.
  - **Tuỳ chọn "dừng service trong lúc backup"** (mặc định tắt, UI cảnh báo có downtime): cờ `stop_service` và thời gian tạm dừng tối đa (mặc định 30 phút) nằm trong mục lịch đã ký gửi tới manager (phase 8), nên chạy được khi control plane offline. **Agent đang chạy scheduler** điều phối toàn bộ, job backup không có quyền gọi Docker API: (1) ghi một lease cục bộ trong bbolt và nhãn service `bn.backup.paused_until=<deadline>`, (2) scale service về 0 và chờ task dừng hẳn, (3) chạy job restic, (4) **luôn** scale về số replica của desired state hiện tại (không phải số đã ghi trước đó) kể cả khi restic lỗi, rồi gỡ lease; kết quả giữ mã thoát của restic. Reconciler trên leader tôn trọng nhãn `bn.backup.paused_until` cho tới deadline mà không cần control plane; quá deadline thì reconciler khôi phục replica và phát `backup.failed`. Deployment của app tới trong lúc có lease sẽ chờ trong hàng đợi tuần tự của app cho tới khi lease được gỡ. Agent chết giữa chừng thì scheduler tiếp quản (phase 8) đọc nhãn, khôi phục replica và báo kết quả. <!-- Updated: Dokploy comparison 2026-11-26 --> <!-- Red Team (Dokploy): 10 -->
- **`internal/agent/jobs` — một quy tắc giữ lại duy nhất** (phase 8 dùng chung): service job đã xong được giữ làm dấu chống chạy trùng trong `max(10 phút, 2 × chu kỳ lịch)` rồi mới xoá. Spec job mang label `bn.job.schedule`, `bn.job.scheduled_at`, `bn.job.deadline` để bất kỳ agent manager nào cũng nhận lại được job. Kết quả gửi với event id = tên service (idempotent). <!-- Red Team: G -->
- `backup_runs(status, size, duration, snapshot_id, log, skipped_reason)`.
- **RBAC/audit/sự kiện:** route database, liên kết, mở public, đích backup, backup tay, restore, recovery kit có `x-permission` + audit; phát `backup.succeeded|failed` qua `events.Emit`. <!-- Red Team: C -->

## Files to Create / Modify
- Create: `images/ops/Dockerfile`, `images/ops/scripts/` (backup/restore theo engine)
- Create: `internal/cp/databases/`, `internal/cp/backups/` (đích, credential → Swarm secret, khoá theo repo, prune, recovery kit)
- Create: `internal/agent/jobs/` (chạy replicated-job, theo dõi, giữ lại theo quy tắc, nhận lại job theo label)
- Create: `migrations/<timestamp>_databases_backups.sql`
- Modify: `internal/cp/servicespec/`, `proto/agent/v1/agent.proto` (job, kết quả job, secret ops), `api/openapi.yaml`, `internal/cp/rbac/policy.go`, `web/src/routes/databases/*`, `web/src/routes/settings/backup-destinations*` <!-- Red Team: N -->

## Tasks & Steps
1. Spec database 4 engine + network riêng + secret.
2. Liên kết app (secret env), mở public có allowlist.
3. Ops image + script.
4. `internal/agent/jobs` với quy tắc giữ lại và nhận lại.
5. Đích backup, credential → Swarm secret, backup tay.
6. Prune tách riêng + khoá theo repo.
7. Restore; recovery kit có RBAC; backup volume (kể cả tuỳ chọn dừng service).
8. RBAC/audit/sự kiện; UI.

## Todo
- [ ] Spec database + network riêng + secret
- [ ] Liên kết app, mở public có allowlist
- [ ] Ops image + script
- [ ] Agent jobs (giữ lại, nhận lại, kết quả idempotent)
- [ ] Đích backup + credential trong Swarm secret
- [ ] Prune riêng + khoá theo repo
- [ ] Restore, recovery kit, backup volume
- [ ] Backup volume có tuỳ chọn dừng service, luôn khôi phục replica
- [ ] RBAC/audit/sự kiện cho route mới
- [ ] UI

## Success Criteria
- Postgres liên kết với app mẫu; app ghi được; app không liên kết không kết nối được tới database.
- Backup Postgres lên MinIO (CI) và R2 (kiểm thật); restore vào database mới; số dòng và checksum bảng khớp.
- Tắt control plane rồi kích hoạt job backup từ agent: backup thành công (credential lấy từ Swarm secret).
- `docker config ls`/`docker service inspect` trên mọi manager không lộ credential S3 hay mật khẩu restic.
- Lệnh dump lỗi giữa chừng: lần backup thất bại và không có snapshot mới.
- `restic check` sau 10 lần backup + prune không lỗi; không có lần nào backup và prune chồng nhau.
- Developer tải recovery kit: 403 và có bản ghi audit bị từ chối.
- Backup volume với tuỳ chọn dừng service bật: service về 0 replica trong lúc backup rồi trở lại đúng số replica của desired state; làm restic lỗi giả: service vẫn được khôi phục và lần backup báo `failed`; kill agent giữa chừng: replica vẫn được khôi phục trước deadline; **tắt control plane** rồi để lịch kích hoạt: backup vẫn nhất quán (service không bị reconciler đẩy lên giữa chừng); push vào app trong lúc backup: deployment chờ tới khi backup xong.
- Tải recovery kit khi phiên đã xác thực hơn 5 phút trước trả `REAUTH_REQUIRED`.

## Verification
- CI: MinIO + 4 engine, test backup → restore từng engine.
- Lab Incus: job Swarm, tắt control plane, kiểm `inspect` không lộ secret.

## Risks
- pg_dump phải cùng hoặc mới hơn server. Phản ứng: client mới nhất; kiểm phiên bản trước khi dump.
- Mất mật khẩu restic là mất backup. Phản ứng: recovery kit + nhắc tải khi tạo đích đầu tiên.
- Prune cần control plane online; control plane offline lâu thì repo phình. Chấp nhận trong v1.
