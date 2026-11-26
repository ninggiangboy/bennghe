---
phase: 3
title: "Cluster Networking"
status: pending
priority: P1
effort: "2.8w"
dependencies: [2]
---

# Phase 3: Cluster Networking

## Goal
Node trong một cụm (có thể khác nhà cung cấp) được nối bằng WireGuard full mesh, chạy Docker Swarm trên IP WireGuard, **sống sót qua reboot khi control plane offline**, chỉ nhận workload sau khi được chuẩn bị xong, có firewall đúng, và được thêm/gỡ an toàn (kể cả node chạy database hay manager). Đây là mốc **alpha1**.

## Requirements
- **Cấp phát IP:** mỗi cụm một /24 từ `10.77.0.0/16` (cấu hình được); địa chỉ trong mesh chỉ dùng IPv4.
- **WireGuard:** agent sinh khoá WG; **private key + danh sách peer gần nhất lưu tại `/var/lib/bennghe-agent/wg/` (0600)**. Agent cấu hình `bn-wg0` bằng crate `wireguard-control` (backend kernel/netlink) và `rtnetlink` cho địa chỉ/route, MTU theo cụm (mặc định 1420). Người dùng có thể ghi đè endpoint. <!-- Red Team: D -->
- **Thứ tự khởi động:** unit oneshot `bn-wg-up.service` dựng lại `bn-wg0` từ file cục bộ, không cần control plane; drop-in `docker.service.d/10-bn-wg.conf` với `After=` và `Requires=bn-wg-up.service`, để dockerd không bao giờ khởi động trước khi IP WG tồn tại (research-01 mục 3). <!-- Red Team: D -->
- **MTU cho mọi overlay:** `daemon.json` trên mọi node đặt `default-network-opts.overlay.com.docker.network.driver.mtu=<MTU cụm>` để mọi overlay — kể cả network do stack tự tạo — kế thừa [UNVERIFIED: kiểm trong lab rằng Docker 29 áp tuỳ chọn này cho overlay]; các network do platform tạo vẫn đặt tường minh. Network `ingress` được tạo lại với cùng MTU ngay sau `swarm init`. **Trước khi join**, agent đo path MTU tới các peer; nếu nhỏ hơn MTU cụm thì từ chối join và chỉ cho phép qua thao tác bảo trì "hạ MTU cụm" (có ghi rõ là gây gián đoạn vì phải tạo lại network). <!-- Red Team: I -->
- **Swarm:** node đầu tiên `swarm init` với `--advertise-addr`, `--data-path-addr`, `--listen-addr` trên IP WG. Join token do agent của leader gửi về (authz phase 2), control plane lưu mã hoá. Mặc định 3 node đầu là manager, sau đó là worker; UI cảnh báo khi số manager chẵn; promote/demote được.
- **Network mặc định:** `bn-system` (cho Traefik, cloudflared, platform). Network theo project và theo database được tạo ở phase 5 và 7 qua `servicespec`. <!-- Red Team: E -->
- **Cổng sẵn sàng:** mọi spec do platform sinh có constraint `node.labels.bn.ready==true`. Agent chỉ đặt nhãn này khi node đã đủ: WG up, firewall áp xong, CA registry đã cài, binary healthcheck/envexec đã có, `/etc/hosts` có tên registry. Thiếu một điều kiện thì gỡ nhãn. <!-- Red Team: D -->
- **Firewall:** backend **iptables** của Docker là bắt buộc; agent phát hiện backend nftables thì **dừng cài và báo lỗi** (fail closed). Chỉ mở 2377/tcp, 7946/tcp+udp, 4789/udp trên `bn-wg0`; mở 51820/udp. Quy tắc trong `DOCKER-USER` khớp `--ctorigdstport` chặn mọi port publish ra internet trừ 80/443 của Traefik và port database được admin mở public (phase 7). <!-- Red Team: N, J -->
- **Nhãn node:** `bn.node=<id control plane>` (agent đặt khi join; dùng để ghim thay cho `node.id`), `bn.ready`, `bn.ingress`, `bn.build`.
- **Gỡ node an toàn:** <!-- Red Team: D -->
  - Kiểm tra trước: node đang chạy database, volume của stack, registry hoặc là build node thì chặn gỡ; UI đề xuất "backup rồi restore sang node X" hoặc bắt gõ tên node để xác nhận mất dữ liệu.
  - Manager được demote trước khi rời; từ chối gỡ nếu cụm ≥ 3 node mà sau khi gỡ còn dưới 3 manager.
  - Drain có timeout (mặc định 10 phút), báo danh sách task bị kẹt.
  - Sau khi rời: xoá node khỏi Swarm, gỡ peer WG ở các node còn lại, thu hồi khoá agent, agent xoá file cert/key của Traefik trên node (nếu từng là ingress).
- **Trạng thái Swarm** (`node ls`, leader) do agent trên leader báo.
- **RBAC/audit:** route promote/demote, nhãn, drain, gỡ node, ghi đè endpoint, bảo trì MTU có `x-permission` (admin cụm) và audit. <!-- Red Team: C -->

## Files to Create / Modify
- Create: `crates/agent/src/wg/`, `crates/agent/src/swarm/`, `crates/agent/src/firewall/`, `crates/agent/src/nodeprep/` (cổng sẵn sàng), `deploy/agent/bn-wg-up.service`, `deploy/agent/docker-bn-wg.conf`, `deploy/agent/daemon.json.tmpl`
- Create: `crates/cp/src/netalloc/`, `crates/cp/src/swarmctl/` (chọn vai trò, quy trình gỡ, kiểm tra trước khi gỡ)
- Create: `migrations/<timestamp>_cluster_network.sql`
- Modify: `proto/agent/v1/agent.proto`, `api/openapi.yaml`, `crates/cp/src/rbac/policy.rs`, `web/src/routes/clusters/*`

## Tasks & Steps
1. Cấp phát IP, lưu khoá WG + peer cục bộ, `bn-wg-up.service`, drop-in docker.
2. Cấu hình WG idempotent bằng `wireguard-control`.
3. `daemon.json` (MTU mặc định overlay, iptables), đo path MTU trước join.
4. Swarm init/join; tạo lại `ingress`; `bn-system`.
5. Firewall iptables + DOCKER-USER, fail closed với nftables.
6. Cổng sẵn sàng `bn.ready`.
7. Promote/demote, nhãn, drain có timeout, gỡ node có kiểm tra trước.
8. UI cụm.

## Todo
- [ ] Khoá WG + peer cục bộ, `bn-wg-up.service`, thứ tự với docker
- [ ] Cấu hình WG idempotent
- [ ] daemon.json + đo path MTU trước join
- [ ] Swarm init/join, tạo lại ingress, bn-system
- [ ] Firewall iptables + DOCKER-USER, fail closed
- [ ] Cổng `bn.ready`
- [ ] Vai trò, nhãn, drain, gỡ node an toàn
- [ ] RBAC/audit cho route mới
- [ ] UI cụm

## Success Criteria
- Cụm 3 node trên 2 nhà cung cấp: ping qua IP WG; `docker node ls` thấy 3 manager.
- **Reboot một manager khi control plane đang tắt:** node quay lại mesh và Swarm mà không cần control plane.
- Test MTU: tải 50 MB giữa container khác node qua `bn-system` **và** qua một overlay do `docker stack deploy` tự tạo, không treo.
- Service thử `replicas=3`; tắt 1 node thì Swarm dời replica trong khoảng 1 phút.
- Node mới join: không task nào được đặt lên trước khi có `bn.ready`.
- Gỡ node đang chạy database bị chặn khi chưa xác nhận.
- nmap từ internet chỉ thấy 51820/udp (và 22 của nhà cung cấp); không thấy 2377/7946/4789/5000 hay port database chưa mở public.

## Verification
- Lab Incus 3 VM (từ phase 1); một lần chạy thật trên 2 nhà cung cấp trước khi đóng phase.

## Risks
- Hai node cùng sau NAT không nối được. Phản ứng: UI báo lỗi rõ; relay ngoài phạm vi v1.
- `default-network-opts` không áp cho overlay như kỳ vọng. Tín hiệu: test tải 50 MB qua overlay của stack treo. Phản ứng: transform ở phase 9 đặt MTU tường minh cho mọi network khai báo và network `default` của stack.
- Swarm: Mirantis gia hạn hỗ trợ tới khoảng 2030, Engine 29.10.1 ra 2026-11-24; rủi ro chiến lược là thương vụ IREN mua Mirantis (2026-08-04). Tín hiệu: thông báo ngừng hỗ trợ. Phản ứng: dừng và lập lại plan cho lớp runtime.
- Mất quorum khi mất 2/3 manager. Phản ứng: runbook `--force-new-cluster` ở phase 12.
