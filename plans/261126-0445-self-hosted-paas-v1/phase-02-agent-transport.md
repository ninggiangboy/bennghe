---
phase: 2
title: "Agent & Transport"
status: pending
priority: P1
effort: "3w"
dependencies: [1]
---

# Phase 2: Agent & Transport

## Goal
Agent Go cài bằng một lệnh, enroll bằng token dùng một lần, xác thực **hai chiều** với control plane qua WebSocket, chỉ áp desired state có chữ ký của control plane, có một reconciler duy nhất cho trạng thái cấp cụm, tự cập nhật an toàn (watchdog, chống hạ cấp, kênh chỉ-cập-nhật cho agent quá cũ), chịu được control plane offline lâu và control plane được restore từ backup.

## Requirements
- **Protocol:** `proto/agent/v1/*.proto` (buf). `Envelope{id, correlation_id, oneof payload}`. Hello mang `agent_version`, `protocol_version`, thông tin node. Control plane hỗ trợ đầy đủ protocol N và N-1. **Agent cũ hơn N-1 vẫn được xác thực nhưng chỉ nhận `SelfUpdate`** (kênh chỉ-cập-nhật đóng băng, mọi phiên bản protocol phải giữ). <!-- Red Team: K -->
- **Kênh:** WebSocket qua TLS dùng `github.com/coder/websocket`; payload protobuf; ping 20 giây. Không dùng gRPC/mTLS: gRPC không đi qua hostname của Cloudflare Tunnel, client cert mTLS không đi qua proxy Cloudflare (research-02 mục 4, 9).
- **Xác thực hai chiều** (theo research-02 mục 9): <!-- Red Team: K -->
  - Agent ký thông điệp có domain separation: `"bn-agent-auth-v1" || nonce || agent_id || server_host || timestamp`.
  - Control plane ký transcript bằng khoá Ed25519 của nó (sinh ở phase 1); agent **ghim fingerprint** khoá này lúc enroll (`--cp-key <fp>` trong lệnh cài) và từ chối control plane không khớp.
  - Mọi `DesiredState` mang `cp_epoch`, `revision` và chữ ký của control plane; agent từ chối bản không có chữ ký hợp lệ.
- **Phân quyền theo loại message** (`internal/cp/agentgw/authz.go`): bảng loại payload × vai trò node × cụm. Join token Swarm chỉ nhận từ node mà control plane đang coi là leader; kết quả job chỉ nhận cho job thuộc cụm của node; thay đổi IP public của node ingress tạo sự kiện chờ admin xác nhận, không tự sửa DNS/peer. Session mới cho cùng node chỉ thay session cũ sau khi bắt tay thành công; đổi session quá 3 lần/phút phát sự kiện cảnh báo. <!-- Red Team: K -->
- **Enroll:** admin tạo join token (dùng một lần, TTL 1 giờ, lưu hash) cho một cụm. Lệnh cài lấy script và binary từ **GitHub Releases** (không phải từ control plane), kiểm SHA-256 + chữ ký minisign bằng public key ghim trong script: `curl -fsSL https://github.com/<org>/bennghe/releases/download/vX/install-agent.sh | sudo sh -s -- --cp https://<cp> --cp-key <fp> --token <t>`. Script cài Docker nếu thiếu (Engine hỗ trợ API ≥ 1.44), wireguard-tools, systemd unit, rồi `agent enroll`. <!-- Red Team: K -->
- **Định danh agent:** khoá Ed25519 chỉ nằm ở file `/var/lib/bennghe-agent/identity.key` (0600) — một nguồn duy nhất, không lưu trong bbolt. <!-- Red Team: scope audit -->
- **Desired state + reconcile:** <!-- Red Team: A, L -->
  - Trạng thái cấp node (WG, firewall, file trên node, container buildkitd/registry) do agent của chính node áp.
  - Trạng thái cấp cụm (service, network, secret, config Swarm) **chỉ** do agent trên node đang là Swarm leader áp; leader đổi thì reconciler chuyển theo. Mọi `ServiceUpdate` gửi kèm version index của spec; xung đột version thì quan sát lại, không retry mù.
  - Trong cùng epoch, agent từ chối revision thấp hơn bản đã áp.
  - **Đổi epoch** (control plane được restore): agent vào chế độ "nhận lại" — gửi toàn bộ observed state, tạm dừng mọi thao tác xoá (gỡ peer, xoá service/secret, thu hồi node) cho tới khi admin xác nhận trên UI.
- **Local state:** bbolt `state.db` chứa outbox và desired state gần nhất. **Outbox có trần dung lượng** (mặc định 256 MB) với 3 lớp ưu tiên: kết quả job + exit code không bao giờ bị bỏ; log bị cắt chỉ giữ đuôi; metrics bị bỏ trước. Agent từ chối khởi chạy job mới khi disk > 95% và ghi sự kiện cục bộ. <!-- Red Team: N -->
- **Reconnect:** exponential backoff có jitter, tối đa 60 giây.
- **Tự cập nhật:** <!-- Red Team: K -->
  - Ký bằng **minisign** (chốt ở phase này, thư viện `aead.dev/minisign`); public key phát hành nhúng trong agent; không cần interface `Verifier`.
  - Chống hạ cấp: từ chối cập nhật về version thấp hơn trừ khi có cờ ghi đè cục bộ trên node.
  - Watchdog trong binary: bản mới phải đạt trạng thái "đã kết nối + đã reconcile" trong 120 giây, nếu không thì tự thoát mã khác 0; systemd `WatchdogSec` + `ExecStartPre` khôi phục bản cũ.
  - Triển khai lần lượt từng node trong cụm, node đang là Swarm leader cập nhật sau cùng.
- **Metrics:** gopsutil + tóm tắt docker stats mỗi 15 giây; control plane giữ raw 24 giờ (biểu đồ 24 giờ trên trang node), không rollup; cảnh báo dùng mẫu mới nhất. <!-- Red Team: O -->
- **RBAC/audit:** route xoá cụm có `reauth: true` (phase 1) <!-- Red Team (Dokploy): 9 -->; mọi route mới (cụm, node, join token, enroll, xác nhận đổi IP, xác nhận nhận lại sau đổi epoch) có `x-permission` và gọi `audit.Record`; phát sự kiện `node.offline`, `node.disk_high` qua `events.Emit`. <!-- Red Team: C -->
- **UI:** danh sách cụm, node (online/offline, version, tài nguyên), hộp thoại "Thêm node" hiện lệnh cài, trang node có biểu đồ 24 giờ, banner chờ xác nhận (đổi IP, đổi epoch).

## Files to Create / Modify
- Create: `proto/agent/v1/agent.proto`, `buf.gen.yaml`, `internal/proto/agentv1/`
- Create: `internal/cp/agentgw/` (WebSocket, handshake hai chiều, authz, session registry, ký desired state), `internal/cp/cluster/`, `internal/cp/node/`, `internal/cp/enroll/`, `internal/cp/metrics/`, `internal/cp/desired/` (dựng + ký desired state, epoch/revision)
- Create: `internal/agent/conn/`, `internal/agent/identity/`, `internal/agent/localstate/` (outbox có trần), `internal/agent/reconcile/` (node-scope + cluster-scope khi là leader), `internal/agent/metrics/`, `internal/agent/selfupdate/` (minisign, watchdog, chống hạ cấp), `internal/agent/dockerx/` (SDK `github.com/moby/moby/client`, ghim phiên bản)
- Create: `deploy/agent/install-agent.sh`, `deploy/agent/bennghe-agent.service`, `.github/workflows/release-agent-dev.yml` (ký minisign các bản dev/alpha)
- Create: `migrations/<timestamp>_clusters_nodes.sql`
- Modify: `api/openapi.yaml`, `internal/cp/rbac/policy.go`, `web/src/routes/clusters/*`

## Tasks & Steps
1. Proto + buf trong CI.
2. Handshake hai chiều + test âm: chữ ký sai, nonce lặp, sai `server_host`, node bị thu hồi, control plane sai fingerprint, protocol N-2 chỉ nhận `SelfUpdate`.
3. Bảng authz theo loại message + test âm (worker gửi join token, node gửi kết quả job của cụm khác).
4. Join token, enroll, install script lấy từ GitHub Releases.
5. Session registry + heartbeat.
6. Desired state có chữ ký, revision, epoch; chế độ nhận lại.
7. Reconciler hai cấp; version index cho cập nhật Swarm.
8. Outbox có trần + watchdog disk.
9. Metrics 24 giờ.
10. Self-update minisign + watchdog + chống hạ cấp + triển khai lần lượt.
11. UI cụm/node + banner xác nhận.

## Todo
- [ ] Proto + codegen
- [ ] Handshake hai chiều + test âm
- [ ] Authz theo loại message
- [ ] Enroll + install script từ GitHub Releases
- [ ] Session registry + heartbeat
- [ ] Desired state ký số, revision, epoch, chế độ nhận lại
- [ ] Reconciler node-scope + cluster-scope trên leader
- [ ] Outbox có trần + watchdog disk
- [ ] Metrics 24 giờ
- [ ] Self-update minisign, watchdog, chống hạ cấp
- [ ] RBAC/audit/sự kiện cho route mới
- [ ] UI cụm/node

## Success Criteria
- Node mới trên VM sạch "online" trong dưới 2 phút từ lúc dán lệnh cài.
- Agent từ chối kết nối tới control plane có khoá không khớp fingerprint đã ghim.
- Tắt control plane 10 phút: sự kiện được đồng bộ đủ, không trùng. Lab: tắt 48 giờ với cron log dày, outbox không vượt trần, kết quả job đủ.
- Agent rảnh dùng dưới 50 MB RAM.
- Bản agent bị treo (không kết nối được) tự quay về bản cũ trong 3 phút; agent N-2 tự nâng cấp qua kênh chỉ-cập-nhật.
- Restore control plane từ backup cũ: agent vào chế độ nhận lại, không peer/service nào bị xoá trước khi admin xác nhận.

## Verification
- `go test ./internal/agent/... ./internal/cp/agentgw/... ./internal/cp/desired/...`
- Integration: control plane + 2 agent trong container.
- Lab Incus (`test/lab/incus-up.sh`): cài, đo thời gian, kịch bản self-update hỏng, kịch bản đổi epoch.

## Risks
- Proxy trước control plane cắt WebSocket idle. Phản ứng: ping 20 giây; tài liệu hoá timeout.
- Agent có quyền Docker socket, tức là root trên node; control plane bị chiếm thì mọi node bị chiếm. Xác thực hai chiều + ký desired state thu hẹp rủi ro này về đúng một điểm: khoá Ed25519 của control plane.
