---
phase: 4
title: "Ingress & Certificates"
status: pending
priority: P1
effort: "2w"
dependencies: [3]
---

# Phase 4: Ingress & Certificates

## Goal
Mỗi cụm có Traefik làm ingress trên các node gắn nhãn ingress, **chỉ đọc cấu hình do agent sinh ra** (không chạm Docker API); control plane cấp và gia hạn chứng chỉ tập trung (HTTP-01 và DNS-01 Cloudflare, có wildcard); domain có chủ sở hữu rõ ràng, không thể bị project khác chiếm; record DNS Cloudflare được tạo tự động mà không bao giờ ghi đè record ngoài hệ thống.

## Requirements
- **Traefik không dùng provider Docker/Swarm:** <!-- Red Team: H -->
  - Traefik v3 chạy Swarm service mode `global`, constraint `node.labels.bn.ingress==true && node.labels.bn.ready==true`, publish 80/443 mode `host`, gắn network `bn-system` và network của các project có domain trong cụm.
  - Chỉ bật **file provider** watch `/etc/bennghe/traefik/dynamic`. Agent trên mỗi node ingress sinh router, service và TLS từ desired state (bảng `domains`): mỗi route trỏ tới `http://<tên-service-swarm>:<port>` qua DNS VIP của Swarm, nên Swarm tự rút task đang dừng khỏi VIP (thay cho label `lbswarm`, và không còn độ trễ poll của provider swarm).
  - Không mount `docker.sock` vào Traefik; node ingress có thể là worker.
  - File YAML chứa PEM cert + key inline, ghi file tạm rồi `rename` nguyên tử trong thư mục watch (Traefik không nạp lại file cert nằm ngoài thư mục này — research-01 mục 5).
- **Cert manager** mở rộng `internal/cp/certs` từ phase 1, dùng `github.com/go-acme/lego/v5` (`Present(ctx, ...)`):
  - Một ACME account mỗi bản cài, khoá lưu mã hoá; bật ARI để gia hạn không tính vào rate limit (50 cert/tên miền đăng ký/tuần, 5 cert trùng/tuần).
  - HTTP-01: token lưu trong Postgres; control plane phục vụ `/.well-known/acme-challenge/{token}`; route ưu tiên cao nhất trong file config của Traefik trỏ path này về URL public của control plane. Route này do platform sinh, người dùng không ghi đè được.
  - DNS-01 qua Cloudflare khi zone đã liên kết; wildcard chỉ qua DNS-01.
  - Gia hạn: River periodic job hằng ngày với `RunOnStart`, khi còn dưới 30 ngày hoặc khi ARI khuyến nghị; lỗi gia hạn gọi `events.Emit(cert.renewal_failed)` (contract từ phase 1, nên cảnh báo hoạt động ngay khi phase 8 thêm kênh). UI hiện cảnh báo từ 21 ngày. <!-- Red Team: C -->
  - Dev/CI dùng Pebble; kiểm thật dùng Let's Encrypt staging.
- **Quyền sở hữu domain:** <!-- Red Team: F -->
  - `domains(id, project_id, owner_type: app|stack_service, owner_id, hostname UNIQUE, tls_mode: http01|dns01|wildcard, cf_zone_id?, proxied, exposure: direct|tunnel)`; `hostname` là duy nhất trong cả bản cài.
  - Admin cấu hình cho mỗi project danh sách zone/hậu tố được phép (ví dụ `*.team-a.example.com`). Domain ngoài danh sách bị từ chối. Domain của control plane luôn bị từ chối.
  - `tls_mode=wildcard` chỉ admin được dùng.
- **Cloudflare DNS:** lưu API token mã hoá + account id (quyền Zone:Read, DNS:Edit; thêm Cloudflare Tunnel:Edit ở phase 10); kiểm quyền token khi liên kết. Khi gắn domain: nếu đã có record trùng tên **không** mang dấu hệ thống (trường comment `managed-by: bennghe`) thì báo xung đột, không ghi đè; admin có thể "nhận quản lý" record đó một cách tường minh (có audit). Tạo record A cho mỗi IP public của node ingress (IP đổi thì chờ admin xác nhận — phase 2). Gỡ domain chỉ xoá record mang dấu hệ thống. <!-- Red Team: F, K -->
- **RBAC/audit:** route Cloudflare (liên kết tài khoản, zone), domain (gắn, gỡ, nhận quản lý record), cấu hình zone của project đều có `x-permission` và audit. <!-- Red Team: C -->
- Control plane phải truy cập được từ internet để HTTP-01 hoạt động; tài liệu hoá.

## Files to Create / Modify
- Create: `internal/cp/certs/` (mở rộng: DNS-01, ARI, gia hạn, phân phối), `internal/cp/cloudflare/` (DNS; tunnel ở phase 10), `internal/cp/domains/` (sở hữu, allowlist zone), `internal/cp/routing/` (mô hình route → desired state cho node ingress)
- Create: `internal/agent/ingress/` (sinh file dynamic config Traefik, ghi nguyên tử, xoá khi bỏ nhãn ingress)
- Create: `migrations/<timestamp>_domains_certs_cloudflare.sql`
- Modify: `internal/cp/servicespec/` được tạo ở phase 5 sẽ dùng `routing`; phase này tạo spec Traefik trong `internal/cp/servicespec/infra.go` (tạo thư mục `servicespec` sớm). <!-- Red Team: N -->
- Modify: `proto/agent/v1/agent.proto`, `api/openapi.yaml`, `internal/cp/rbac/policy.go`, `web/src/routes/settings/cloudflare*`, `web/src/routes/clusters/*`

## Tasks & Steps
1. Spec Traefik (file provider only) trong `servicespec/infra.go`.
2. Agent sinh file dynamic config; route challenge; redirect HTTP→HTTPS.
3. Mở rộng cert manager: DNS-01, ARI, gia hạn, sự kiện.
4. Mô hình domain + allowlist zone theo project + chặn domain control plane.
5. Cloudflare DNS: không ghi đè record ngoài hệ thống, nhận quản lý tường minh.
6. UI.

## Todo
- [ ] Spec Traefik file-provider-only
- [ ] Agent sinh dynamic config, ghi nguyên tử
- [ ] DNS-01, ARI, gia hạn, sự kiện
- [ ] Sở hữu domain + allowlist zone
- [ ] Cloudflare DNS an toàn
- [ ] RBAC/audit cho route mới
- [ ] UI

## Success Criteria
- Cert staging qua HTTP-01; HTTPS hoạt động trên cả hai node ingress, trong đó một node là worker.
- Cert `*.example-test.com` qua DNS-01 Cloudflare (admin).
- Developer gắn domain ngoài allowlist của project, hoặc domain của control plane, hoặc hostname đã thuộc project khác: bị từ chối.
- Record A có sẵn tạo tay trên Cloudflare không bị ghi đè hay xoá.
- Ép gia hạn: cert mới có hiệu lực mà không restart Traefik.

## Verification
- Unit test HTTP-01 provider, chọn tls_mode, allowlist; golden test file config Traefik.
- Pebble trong CI; kiểm thật với zone Cloudflare thử.

## Risks
- Traefik phải gắn thêm network mỗi khi có project mới trong cụm, gây rolling restart Traefik (port mode host nên mỗi node gián đoạn vài giây). Người dùng đã chấp nhận đánh đổi này (Validation Session 1). Phản ứng: parallelism 1, delay 30 giây; restart chỉ xảy ra khi một project có domain đầu tiên trong cụm. <!-- Updated: Validation Session 1 - chấp nhận restart Traefik -->
- Control plane chết lâu hơn khoảng gia hạn thì cert hết hạn. Phản ứng: cảnh báo từ 21 ngày.
