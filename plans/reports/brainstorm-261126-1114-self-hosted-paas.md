# Brainstorm — Self-hosted PaaS quản lý nhiều cụm VPS

Ngày: 2026-11-26 · Trạng thái: chờ chốt mô hình cụm → chuyển `/ak-plan`

## Quyết định đã chốt (từ người dùng)

- Sản phẩm **open source, self-hosted**: mỗi người/tổ chức tự cài một control plane cho mình (single-tenant).
- Hỗ trợ **nhiều cụm VPS** (cluster) trong một control plane.
- Stack: **Go** (control plane + agent) và **React + Vite** (web UI).
- Kiến trúc: **Control plane + agent** (agent chủ động kết nối ra, không dùng SSH).

## Contract

**Outcome:** Người dùng cài control plane bằng một lệnh, gắn VPS vào các cụm, rồi từ web UI có thể deploy app (Git / Dockerfile / image / compose) kèm domain + SSL, tạo database, chạy cron job, backup lên S3 và restore được — trên nhiều cụm.

**Constraints:**
- Agent là một binary Go tĩnh, chạy ổn trên VPS 1 GB RAM, chỉ cần kết nối outbound tới control plane (chạy được sau NAT).
- Kênh agent ↔ control plane mã hoá mTLS; không lưu SSH key; secret (env, mật khẩu DB, khoá S3) mã hoá at-rest.
- Deploy lỗi không được làm sập phiên bản đang chạy.
- Cron và backup đã lên lịch vẫn chạy khi control plane tạm offline.
- Protocol agent ↔ control plane có version ngay từ đầu.
- Cài đặt đơn giản: control plane là 1 binary nhúng sẵn SPA, chạy bằng docker compose cùng Postgres.

**Non-goals (v1):**
- Multi-tenant SaaS, billing, đăng ký công khai.
- Kubernetes, autoscaling.
- Tự quản lý DNS provider (người dùng tự trỏ record).
- Replica một service chạy trên nhiều node + load balancing giữa node (để sau, xem câu hỏi mở).

**Acceptance criteria:**
1. Cài control plane bằng một lệnh; đăng ký VPS mới vào một cụm trong dưới 2 phút bằng token dùng một lần.
2. Push Git → app tự build và deploy, có SSL tự động, không downtime (healthcheck đạt mới chuyển traffic).
3. Deploy lỗi giữ nguyên bản cũ; rollback về bản trước bằng 1 thao tác.
4. Tạo Postgres/MySQL/Redis bằng 1 thao tác; app trong cùng cụm kết nối được qua mạng private, kể cả khác node.
5. Backup DB theo lịch lên S3-compatible (mã hoá, có retention); restore thành công vào một DB mới.
6. Cron chạy đúng lịch, lưu lịch sử/exit code/log; lỗi thì gửi thông báo (Telegram/Slack/email/webhook).
7. Ngắt control plane 10 phút: app, cron, backup trên node vẫn chạy; agent tự kết nối lại và đồng bộ kết quả.

## Kiến trúc đề xuất

```
                ┌──────────── Control plane (Go) ────────────┐
 Browser ──────▶│ HTTP API + WebSocket │ React SPA (embedded)│
                │ Job queue (Postgres) │ CA cấp cert agent   │
                └───────────────▲────────────────────────────┘
                                │ gRPC bidi stream, mTLS (agent dial ra)
          ┌─────────────────────┼─────────────────────┐
     Cụm A: node1  node2 ◀─ WireGuard mesh ─▶      Cụm B: node3 ...
     mỗi node: agent ─▶ Docker Engine (unix socket)
               Caddy (agent cấu hình qua admin API)
               scheduler cục bộ (cron/backup, state lưu local)
```

- **Control plane:** Go, Postgres, job queue trên Postgres (ví dụ River), API REST + WebSocket cho log/metrics realtime.
- **Agent:** systemd service; enrollment bằng token một lần → sinh keypair, CSR được CA của control plane ký; điều khiển Docker qua SDK; tự cập nhật theo version control plane.
- **Cụm:** nhóm node logic + mạng private WireGuard giữa các node trong cụm (control plane phân phối peer config). v1: mỗi service gắn vào một node do người dùng chọn.
- **Proxy:** Caddy trên mỗi node có nhận traffic, SSL tự động.
- **Build:** mặc định build trên node đích; cho phép chọn build node riêng trong cụm hoặc dùng image từ registry (CI build sẵn) để không làm nghẽn VPS nhỏ.
- **Backup:** dump DB (pg_dump/mysqldump…) → nén → mã hoá → S3; volume dùng restic (dedupe, mã hoá, retention có sẵn).

## Lộ trình MVP

1. Nền tảng: control plane skeleton, auth, cụm/node, agent enrollment + mTLS stream, metrics, danh sách container/log.
2. Deploy: image/Dockerfile/Git webhook, env/secret, domain + SSL qua Caddy, zero-downtime, rollback.
3. Mạng cụm (WireGuard) + database một click.
4. Backup S3 + restore; cron + thông báo.
5. Docker-compose, build node/registry, team + RBAC, audit log.

## Trade-offs

**Mô hình cụm (cần chốt):**

| Hướng | Giả định chính | Hỏng đầu tiên khi |
|---|---|---|
| **1. Docker thường + WireGuard, service gắn 1 node (khuyến nghị v1)** | Người dùng chấp nhận chọn node cho từng service | Cần replica trên nhiều node để HA — phải tự viết scheduler sau |
| **2. Docker Swarm mỗi cụm** (như Dokploy) | Swarm còn được duy trì đủ lâu | Lỗi Swarm (overlay network, quorum manager) khó debug; bị khoá vào hệ sinh thái Swarm |
| **3. Tự viết scheduler + replica đa node ngay** | Có đủ thời gian cho phần khó nhất | MVP kéo dài nhiều tháng trước khi có thứ dùng được |

Hướng 1 rẻ nhất để bỏ: lớp placement để thành interface, sau này thêm replica đa node mà không đổi model dữ liệu chính.

**Agent vs SSH:** đã chọn agent — đổi lại phải duy trì cơ chế tự cập nhật agent và tương thích protocol.

## Better approaches

Coolify, Dokploy, CapRover, Dokku đã có sẵn. Mục tiêu là sản phẩm open source nên việc tự build hợp lý, nhưng README cần nêu rõ điểm khác biệt. Ứng viên từ kiến trúc đã chọn: agent pull-based (không lưu SSH key, chạy sau NAT), multi-cụm là khái niệm hạng nhất, footprint nhẹ (agent Go tĩnh), cron/backup chạy cả khi control plane offline.

## Câu hỏi mở

1. Mô hình cụm v1: hướng 1 (khuyến nghị), 2 hay 3?
2. Giấy phép: AGPL-3.0 (chặn người khác đóng gói thành SaaS đóng) hay Apache-2.0/MIT (dễ được chấp nhận hơn)?
