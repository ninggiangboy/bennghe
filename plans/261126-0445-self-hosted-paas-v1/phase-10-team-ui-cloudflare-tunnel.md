---
phase: 10
title: "Team UI & Cloudflare Tunnel"
status: pending
priority: P2
effort: "2w"
dependencies: [4, 8]
---

# Phase 10: Team UI & Cloudflare Tunnel

## Goal
Quản lý team (mời, phân vai trò theo cụm/project, gỡ người), xem audit log; và expose app qua Cloudflare Tunnel cho cụm không mở 80/443.

## Requirements
- **Team:** mời bằng link token (gửi email nếu đã cấu hình SMTP ở phase 8), chấp nhận lời mời thì tạo tài khoản; đổi vai trò; gỡ thành viên (huỷ session của họ ngay). Không cho gỡ owner cuối cùng.
- **Audit UI:** danh sách có lọc theo người, hành động, kết quả (thành công/bị từ chối), đối tượng, thời gian. (Bỏ xuất CSV — Red Team O.)
- **Kiểm RBAC toàn bộ:** cổng `TestAllRoutes` đã chạy từ phase 1; phase này bổ sung chiều nội dung spec (developer không tạo được workload vi phạm chính sách kiểm duyệt qua bất kỳ route nào: app, cron, stack, template). <!-- Red Team: C, E -->
- **Cloudflare Tunnel** (mỗi cụm tuỳ chọn bật):
  - Control plane tạo tunnel loại remotely-managed qua API, lưu token mã hoá. Cấu hình ingress đặt bằng `PUT /accounts/{id}/cfd_tunnel/{id}/configurations`; lệnh PUT thay toàn bộ danh sách và phải kết thúc bằng rule catch-all (`http_status:404`), nên control plane luôn dựng lại cả danh sách từ bảng domains thay vì sửa từng phần.
  - Deploy `cloudflared` thành Swarm service 2 replica, gắn overlay network chung với Traefik, token qua Swarm secret.
  - Domain có `exposure=tunnel`: control plane thêm hostname vào cấu hình ingress của tunnel (trỏ tới Traefik trên overlay) và tạo CNAME `<tunnel-id>.cfargotunnel.com` có proxy.
  - Route cho host đó do agent ingress sinh trong file config Traefik (cùng cơ chế phase 4), trên entrypoint HTTP nội bộ, không redirect HTTPS (Cloudflare đã terminate TLS). cloudflared gắn `bn-system` và gọi Traefik qua tên service. <!-- Red Team: H -->
- Preset R2 đã có ở phase 7; phase này chỉ kiểm lại luồng Cloudflare đầy đủ.

## Files to Create / Modify
- Create: `internal/cp/team/`, `internal/cp/cloudflare/tunnel.go`, `internal/cp/tunnel/`
- Create: `migrations/00010_invites_tunnels.sql`
- Modify: `internal/cp/rbac/`, `internal/cp/servicespec/` (spec cloudflared), `internal/cp/routing/` (route chế độ tunnel), `internal/agent/ingress/`, `api/openapi.yaml`, `web/src/routes/settings/team*`, `web/src/routes/audit*`, `web/src/routes/clusters/*` (bật tunnel)

## Tasks & Steps
1. Lời mời + chấp nhận + đổi vai trò + gỡ người.
2. Audit UI + CSV.
3. Test RBAC duyệt toàn bộ route.
4. Tunnel: tạo qua API, service cloudflared, cấu hình ingress, CNAME.
5. Router Traefik cho chế độ tunnel.

## Todo
- [ ] Lời mời, vai trò, gỡ người
- [ ] Audit UI + CSV
- [ ] Test RBAC toàn route
- [ ] Tunnel API + cloudflared service
- [ ] Ingress tunnel + CNAME + router

## Success Criteria
- Viewer bấm Deploy bị chặn (403) và audit log ghi lần bị từ chối (cơ chế `RecordDenied` từ phase 1).
- Test RBAC toàn route xanh, không route nào thiếu kiểm quyền.
- Cụm đóng hết 80/443 bằng firewall nhà cung cấp: app có `exposure=tunnel` truy cập HTTPS được qua domain.

## Verification
- `go test ./internal/cp/rbac/... -run TestAllRoutes`
- Kiểm thật tunnel với zone Cloudflare thử.

## Risks
- Token Cloudflare thiếu quyền (Tunnel, DNS). Phản ứng: kiểm quyền token khi liên kết và báo thiếu quyền nào.
