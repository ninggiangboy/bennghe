---
phase: 8
title: "Scheduler, Cron & Notifications"
status: pending
priority: P1
effort: "2w"
dependencies: [7]
---

# Phase 8: Scheduler, Cron & Notifications

## Goal
Cron job và lịch backup chạy đúng giờ khi control plane offline, chịu được đổi leader và agent leader chết, không chạy trùng; có lịch sử, log và thông báo qua Telegram, Slack, email, webhook. Đây là mốc **alpha3**.

## Requirements
- **Lưu lịch qua desired state, không dùng Swarm config:** control plane gửi danh sách lịch của cụm (đã ký, theo phase 2) tới agent của **mọi manager**; mỗi agent lưu trong bbolt. Lịch chỉ chứa tham chiếu tên secret, không chứa credential. <!-- Red Team: G, B -->
- **Ai chạy scheduler:** <!-- Red Team: G -->
  - Agent trên node đang là Swarm leader chạy scheduler (`robfig/cron/v3`, có timezone) và ghi nhịp tim vào nhãn node `bn.sched.heartbeat` mỗi 15 giây.
  - Nếu nhịp tim cũ hơn 45 giây (agent leader chết, đang tự cập nhật hoặc crash-loop) thì agent manager khoẻ có `bn.node` nhỏ nhất tiếp quản cho tới khi nhịp tim của leader trở lại.
  - Tự cập nhật agent: node leader cập nhật sau cùng (phase 2).
- **Chống chạy trùng:** tên service = `bn-cron-<schedule-id>-<scheduled-unix>`, trong đó `scheduled-unix` là **thời điểm theo biểu thức cron**, không phải đồng hồ hiện tại. Service đã xong được giữ theo quy tắc phase 7 (`max(10 phút, 2 × chu kỳ)`), nên hai scheduler chạy chồng trong lúc chuyển giao không tạo được tên trùng. Không chạy bù lượt bị lỡ (ghi trong tài liệu). <!-- Red Team: G -->
- **Đồng hồ:** agent kiểm lệch giờ so với control plane và các manager khác; lệch > 2 giây phát sự kiện và hiện cảnh báo; tài liệu yêu cầu NTP. <!-- Red Team: G -->
- **Nhận lại job khi đổi leader:** scheduler mới quét service `bn-cron-*`/`bn-backup-*` theo label (deadline, schedule) và tiếp quản timeout, thu kết quả, dọn dẹp. Mọi agent manager đều báo được kết quả (event id = tên service). <!-- Red Team: G -->
- **Thực thi** (`internal/agent/jobs` + `servicespec`): `replicated-job` với image hiện tại của app (cùng network; env của app chỉ được kế thừa khi người dùng bật) hoặc image tuỳ ý (không kế thừa env, đi qua chính sách kiểm duyệt); lệnh; timeout; chính sách song song `forbid | allow`; `MaxAttempts` tường minh. <!-- Red Team: M, E -->
- **Kết quả:** exit code, thời gian, log (cắt 1 MB) vào outbox (lớp ưu tiên của phase 2), đồng bộ lên control plane.
- Lịch backup phase 7 dùng cùng cơ chế (loại job `backup`), tôn trọng khoá theo repo.
- **Thông báo** — chỉ thêm phần tiêu thụ cho contract `events.Emit` từ phase 1: kênh `telegram | slack | email (SMTP) | webhook (URL + HMAC)` cấu hình mã hoá, URL webhook đi qua `netpolicy`. Quy tắc theo sự kiện đã được các phase trước phát: deploy, cron, backup, node offline > 2 phút, disk > 90%, gia hạn cert thất bại, lệch giờ, đổi session agent bất thường. Khi control plane offline, thông báo về các lượt chạy đi muộn sau khi đồng bộ (tài liệu). <!-- Red Team: C -->
- **RBAC/audit:** route cron, kênh, quy tắc, gửi thử có `x-permission` + audit.
- **UI:** cron theo app (xem trước 5 lần chạy kế), lịch sử + log; kênh và quy tắc thông báo, nút gửi thử.

## Files to Create / Modify
- Create: `internal/agent/scheduler/` (leader + tiếp quản theo nhịp tim, nhận lại job, kiểm lệch giờ), `internal/cp/cron/`, `internal/cp/notify/` (một file mỗi kênh, tiêu thụ River job từ `events`)
- Create: `migrations/<timestamp>_cron_notifications.sql`
- Modify: `internal/cp/desired/` (lịch trong desired state cho manager), `internal/cp/backups/`, `internal/cp/servicespec/`, `proto/agent/v1/agent.proto`, `api/openapi.yaml`, `internal/cp/rbac/policy.go`, `web/src/routes/apps/*/cron*`, `web/src/routes/settings/notifications*` <!-- Red Team: N -->

## Tasks & Steps
1. Lịch trong desired state tới mọi manager.
2. Scheduler trên leader + nhịp tim + tiếp quản.
3. Tên job theo thời điểm lịch; nhận lại job; kiểm lệch giờ.
4. Thực thi + kết quả.
5. Lịch backup.
6. Kênh + quy tắc thông báo.
7. RBAC/audit; UI.

## Todo
- [ ] Lịch qua desired state tới mọi manager
- [ ] Scheduler leader + nhịp tim + tiếp quản
- [ ] Tên job theo thời điểm lịch, nhận lại job, kiểm lệch giờ
- [ ] Thực thi + kết quả
- [ ] Lịch backup
- [ ] 4 kênh thông báo + quy tắc
- [ ] RBAC/audit cho route mới
- [ ] UI

## Success Criteria
- Cron mỗi phút trong 1 giờ: đúng 60 lần, không trùng.
- Tắt node leader: lượt kế tiếp chạy trên leader mới, không trùng, lỡ tối đa một lượt; job đang chạy của leader cũ vẫn có kết quả.
- Dừng process agent trên leader (dockerd vẫn chạy): agent manager khác tiếp quản trong dưới 1 phút.
- Tắt control plane 10 phút: 10 lượt vẫn chạy; bật lại có đủ 10 bản ghi lịch sử.
- Cron exit 1 thì Telegram nhận thông báo trong 1 phút (khi control plane online).

## Verification
- Unit test scheduler với clock giả; lab Incus 3 manager: tắt leader, dừng agent leader, tắt control plane, lệch giờ giả.

## Risks
- Leader đổi đúng phút chạy dẫn đến lỡ một lượt. Chấp nhận, ghi tài liệu.
- Nhãn nhịp tim ghi vào Raft mỗi 15 giây. Tín hiệu: Raft chậm trên cụm lớn. Phản ứng: tăng chu kỳ lên 30 giây.
