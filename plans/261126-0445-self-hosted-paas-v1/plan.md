---
title: "Self-hosted PaaS v1"
description: "PaaS mã nguồn mở (AGPL-3.0) tự cài: một control plane Go + React quản lý nhiều cụm Docker Swarm trên WireGuard, deploy app/compose, database, backup S3/R2, cron, team/RBAC và tích hợp Cloudflare."
status: pending
priority: P1
effort: "47w"
tags: [feature, infra, backend, frontend, api]
blockedBy: []
blocks: []
created: 2026-11-26
---

# Self-hosted PaaS v1

## Overview

PaaS mã nguồn mở (AGPL-3.0) tự cài: một control plane Go + React quản lý nhiều cụm Docker Swarm trên WireGuard, deploy app/compose, database, backup S3/R2, cron, team/RBAC và tích hợp Cloudflare. Bản HTML đầy đủ: [plan.html](./plan.html) (chưa cập nhật phần bổ sung từ Dokploy ngày 2026-11-26; `plan.md` và các phase file là nguồn chính).

Nguồn: [brainstorm](../reports/brainstorm-261126-1114-self-hosted-paas.md), phần tư vấn đã xác nhận ngày 2026-11-26, research: [hạ tầng](./research/researcher-01-infra-runtime-report.md), [tích hợp](./research/researcher-02-integrations-report.md).

Effort: tổng các phase 39,1 tuần + 20% buffer tích hợp và ổn định ≈ 47 tuần cho **1 người full-time** (khoảng 11 tháng; alpha1 ~tuần 12, alpha2 ~tuần 23, alpha3 ~tuần 34). Các phase chạy tuần tự. Trước bổ sung từ so sánh Dokploy (2026-11-26) là 29,5 + buffer ≈ 36 tuần. <!-- Updated: Dokploy comparison 2026-11-26 -->

## Key Decisions (đã chốt với người dùng)

- Tên sản phẩm **Bennghe** (từ Bến Nghé — bến cảng xưa của Sài Gòn); module `github.com/<org>/bennghe`; prefix kỹ thuật `bn` (nhãn `bn.*`, interface `bn-wg0`, đường dẫn `/var/lib/bennghe-agent`). Cần kiểm tra tên còn trống trên GitHub/domain trước khi public.
- Open source, self-hosted, single-tenant; một control plane quản lý nhiều cụm. AGPL-3.0.
- Go cho control plane + agent; React + Vite cho UI (nhúng vào binary).
- Control plane + agent; agent kết nối ra (WebSocket + TLS), xác thực hai chiều Ed25519, desired state có chữ ký.
- Mỗi cụm là Docker Swarm trên WireGuard full mesh; node có thể khác nhà cung cấp.
- Registry nội bộ mỗi cụm + build node.
- Git native: GitHub App (GitHub.com + GHES), GitLab, Gitea; Forgejo best-effort qua adapter Gitea.
- Cloudflare: DNS tự động, DNS-01/wildcard, Tunnel, preset R2.
- Toàn bộ 12 nhóm tính năng trong v1 (scope: HOLD).
- **Project là ranh giới cách ly bảo mật thật** trong một cụm (chốt ở red-team 2026-11-26).
- **Bổ sung từ so sánh Dokploy** (người dùng chọn 2026-11-26, [báo cáo](../reports/xia-261126-1619-dokploy-comparison.md)): 2FA TOTP, xác thực lại 5 phút cho thao tác nhạy cảm, API token cho CI (phase 1); cert tải lên `tls_mode=custom` (phase 4); huỷ deployment, stream log build, giới hạn build song song, dọn Docker trên node, biến `${{project.KEY}}` (phase 5); watch paths (phase 6); backup volume có dừng service (phase 7); cảnh báo CPU/RAM (phase 8); generator base64/uuid/jwt (phase 9); **preview deployment theo PR** thành phase 11 mới, E2E & Release chuyển thành phase 12. Chỉ lấy ý tưởng; không chép mã, không dùng phần `/proprietary` của Dokploy.

## Phases

| # | Phase | Status | Depends on |
|---|-------|--------|------------|
| 1 | [Foundation](./phase-01-foundation.md) | Pending | — |
| 2 | [Agent & Transport](./phase-02-agent-transport.md) | Pending | 1 |
| 3 | [Cluster Networking](./phase-03-cluster-networking.md) — alpha1 | Pending | 2 |
| 4 | [Ingress & Certificates](./phase-04-ingress-certificates.md) | Pending | 3 |
| 5 | [Build & Deploy](./phase-05-build-deploy.md) — alpha2 | Pending | 4 |
| 6 | [Git Providers](./phase-06-git-providers.md) | Pending | 5 |
| 7 | [Databases & Backups](./phase-07-databases-backups.md) | Pending | 5 |
| 8 | [Scheduler, Cron & Notifications](./phase-08-scheduler-cron-notifications.md) — alpha3 | Pending | 7 |
| 9 | [Compose & Templates](./phase-09-compose-templates.md) | Pending | 6, 7 |
| 10 | [Team UI & Cloudflare Tunnel](./phase-10-team-ui-cloudflare-tunnel.md) | Pending | 4, 8 |
| 11 | [Preview Deployments](./phase-11-preview-deployments.md) | Pending | 4–8, 10 |
| 12 | [E2E & Release](./phase-12-e2e-release.md) — v1.0 | Pending | 1–11 |

## Success Criteria (Done means)

- [ ] Cài control plane bằng một lệnh; node join vào cụm dưới 2 phút.
- [ ] Cụm 3 node trên 2 nhà cung cấp chạy app `replicas=3`; tắt 1 node thì Swarm dời replica.
- [ ] Push lên GitHub/GitLab/Gitea thì app tự build, deploy với 0 request lỗi, commit status cập nhật.
- [ ] Deploy lỗi giữ nguyên bản cũ; rollback tay một thao tác trả lại cả image lẫn cấu hình.
- [ ] Postgres backup theo lịch lên S3 và R2; restore vào database mới thành công.
- [ ] Cron lỗi gửi thông báo.
- [ ] Deploy được template compose n8n.
- [ ] Viewer bị chặn deploy và audit log ghi lại.
- [ ] Gắn domain tự tạo record Cloudflare; cấp được cert wildcard; app truy cập qua Tunnel khi đóng 80/443.
- [ ] Tắt control plane 10 phút: app, cron, backup vẫn chạy; agent tự kết nối lại và đồng bộ.
- [ ] Owner/admin đăng nhập bằng 2FA; tải recovery kit đòi xác thực lại; CI deploy được bằng API token có scope project.
- [ ] PR của collaborator có preview HTTPS riêng không chạm secret hay network production; PR từ fork của người không có quyền ghi không được build; đóng PR thì preview bị gỡ.
- [ ] Push chỉ sửa file ngoài `watch_paths` không tạo deployment; dọn Docker giữ đĩa node dưới ngưỡng mà không xoá image đang cần.

## Out of Scope

SaaS đa khách hàng, billing, Kubernetes, autoscaling, DNS provider ngoài Cloudflare, HA cho control plane/database/registry, relay cho node sau NAT hai phía, Prometheus/Loki/Grafana, xuất CSV audit, rollup metrics dài hạn; preview cho stack compose và sao chép database cho preview; các mục Dokploy không được chọn (backup tự động control plane, middleware redirect/basic auth, terminal container, kiểm tra bảo mật node, kênh Discord/ntfy) — ứng viên v1.1.

## Constraints

Go + React/Vite; AGPL-3.0; control plane và Traefik không mount Docker socket; agent chỉ cần kết nối ra; node trong cụm cần UDP 51820 reachable; secret mã hoá at-rest và không bao giờ nằm trong Swarm config/env thường; protocol agent có version (N, N-1 đầy đủ; cũ hơn chỉ nhận cập nhật); MTU overlay đặt tường minh; database và volume của stack ghim node, stop-first; Docker dùng backend iptables.

## Load-bearing Assumptions

| Giả định | Tín hiệu hỏng | Phản ứng định trước |
|---|---|---|
| Swarm còn được duy trì trong vòng đời v1 (đã xác nhận: Mirantis hỗ trợ tới khoảng 2030, Engine 29.10.1 ra 2026-11-24) | Thông báo ngừng hỗ trợ sau thương vụ IREN mua Mirantis | Dừng và lập lại plan cho lớp runtime |
| Rolling update 0 request lỗi (Swarm chờ HEALTHCHECK đã xác nhận; Traefik route tới VIP của service) | Test k6 phase 5 có request lỗi | Thêm retry middleware cho request idempotent; nếu vẫn lỗi thì hỏi người dùng hạ mục tiêu |
| MTU wg0 1420 phù hợp mọi nhà cung cấp; `default-network-opts` áp cho mọi overlay | Test tải 50 MB phase 3/9 treo | Đo path MTU trước khi join; transform đặt MTU tường minh |
| Gắn Traefik vào network của từng project chấp nhận được (restart Traefik khi có project mới; thêm một lần khi project bật preview — phase 11) | Gián đoạn đo được ở phase 4 | Chuyển sang network ingress chung có kiểm soát; hỏi người dùng |

## Dependencies

Docker Engine (Swarm mode, API ≥ 1.44, backend iptables), WireGuard, Traefik v3 (file provider), distribution registry, BuildKit, Railpack, lego v5, River, restic, compose-go, minisign, Cloudflare API, GitHub/GitLab/Gitea API.

## Red Team Review

### Session — 2026-11-26
**Findings:** 40 thô từ 4 reviewer, gộp thành 15 nhóm (15 accepted, 0 rejected; một ý con trong nhóm O bị bác: bỏ tuỳ chọn public database, vì người dùng đã xác nhận tính năng này)
**Severity breakdown:** 6 Critical, 8 High, 1 Medium

| # | Finding | Severity | Disposition | Applied To |
|---|---------|----------|-------------|------------|
| A | Deploy cùng app không tuần tự; reconcile đánh nhau với rollback Swarm; rollback tay thiếu cấu hình | Critical | Accept | Phase 2, 5, 6, 9 |
| B | Credential backup khi control plane offline; credential xoá được toàn bucket; recovery kit không RBAC | Critical | Accept | Phase 7, 8, 9 |
| C | RBAC/audit/sự kiện phải nối từ phase 1; audit lệnh bị từ chối | Critical | Accept | Phase 1–10 |
| D | Reboot khi control plane offline; cổng sẵn sàng node; gỡ node chạy database/manager | Critical | Accept | Phase 2, 3 |
| E | Không có chính sách kiểm duyệt workload; không cách ly project | Critical | Accept (người dùng chọn cách ly thật) | Phase 3, 5, 7, 8, 9, 10 |
| F | Chiếm domain, ghi đè DNS, label Traefik tự viết | Critical | Accept | Phase 4, 5, 9 |
| G | Cron: liveness agent leader, chống trùng, nhận lại job; bỏ Swarm config | High | Accept | Phase 7, 8 |
| H | Traefik provider swarm cần socket manager → agent sinh file config | High | Accept | Phase 4, 5, 10 |
| I | MTU cho mọi overlay; đo trước khi join | High | Accept | Phase 3, 9 |
| J | Registry gắn IP, publish 0.0.0.0, tag thay vì digest, dọn nhầm image | High | Accept | Phase 3, 5 |
| K | Xác thực hai chiều, authz theo message, chống hạ cấp, kênh chỉ-cập-nhật, ký ở phase 2 | High | Accept | Phase 2, 3, 4, 11 (nay là 12) |
| L | Restore control plane làm lùi revision (epoch); nâng cấp có pg_dump | High | Accept | Phase 2, 11 (nay là 12) |
| M | Token trong log, secret env, replay webhook, OAuth state, SSRF, setup token | High | Accept | Phase 1, 5, 6, 8 |
| N | Cấu trúc: servicespec duy nhất, volume stack, file list, migration timestamp, lab sớm, bỏ Caddy, outbox có trần, ước lượng | High | Accept | Nhiều phase, plan.md |
| O | Tính năng ngoài yêu cầu (CSV, rollup, GHES) | Medium | Accept một phần (GHES → validate) | Phase 2, 6, 10 |

### Whole-Plan Consistency Sweep
- Files reread: plan.md, phase-01 … phase-11.
- Decision deltas checked: 14 (Caddy → TLS tự phục vụ; migration timestamp; minisign ở phase 2; Traefik file provider; registry container + tên ổn định + token auth; Swarm config lịch → desired state; quy tắc giữ job thống nhất; network theo project/database; domains có owner + unique; event contract phase 1; RecordDenied; lab ở phase 1; effort 22w → 35w; phụ thuộc phase 9 → 6, 7).
- Reconciled stale references: 9 (nftables wording phase 3; `lbswarm` UNVERIFIED phase 5; "Verifier" phase 2; lab trong phase 11; CSV phase 10; router tunnel qua servicespec phase 10; Swarm config phase 8; "app đang chạy không ảnh hưởng" phase 5; tham chiếu Caddy phase 1).
- Unresolved contradictions: 0.

### Session — 2026-11-26 (bổ sung từ so sánh Dokploy)
**Phạm vi:** chỉ phần bổ sung đánh dấu `Updated: Dokploy comparison 2026-11-26`, trọng tâm phase 11 (preview).
**Findings:** 40 thô từ 4 reviewer, gộp thành 15 (15 accepted, 0 rejected; người dùng chọn áp dụng cả 15). Marker trong phase: `Red Team (Dokploy): n`.
**Severity breakdown:** 3 Critical, 9 High, 3 Medium

| # | Finding | Severity | Disposition | Applied To |
|---|---------|----------|-------------|------------|
| 1 | `deployments` không phân biệt preview/production: rollback, supersede, giữ secret, base watch paths có thể chọn snapshot của PR | Critical | Accept | Phase 5, 11 |
| 2 | Cổng PR kiểm tác giả chứ không kiểm người đẩy; build ref di động thay vì SHA đã kiểm; cache quyền mâu thuẫn | Critical | Accept | Phase 11 |
| 3 | Build preview dùng chung cache/secret build với production | Critical | Accept | Phase 11 |
| 4 | Preview zone một wildcard cho cả project nhiều cụm; Tunnel lộ IP và vượt độ sâu Universal SSL; dòng domain preview đi qua luồng cert/DNS từng domain | High | Accept | Phase 4, 11 |
| 5 | Webhook phase 6 không đăng ký/không xử lý sự kiện PR; khoá khử trùng nuốt sự kiện PR | High | Accept | Phase 6, 11 |
| 6 | Gỡ preview đua với deployment, không dọn tài nguyên zone, quét dọn không xử lý lỗi | High | Accept | Phase 11 |
| 7 | Ngoại lệ `${{project.X}}` cho preview có thể lộ secret production | High | Accept | Phase 11 |
| 8 | Cert tải lên có SAN của project khác chiếm TLS của họ | High | Accept | Phase 4 |
| 9 | API token giữ vai trò cũ, né 2FA bắt buộc, scope chưa đủ; danh sách `reauth` không có chủ; owner mất 2FA không có lối thoát | High | Accept | Phase 1, 2, 5, 7, 9, 12 |
| 10 | Backup volume dừng service cần desired state có chữ ký nên hỏng khi control plane offline | High | Accept | Phase 7, 8 |
| 11 | Hàng đợi build thiếu bộ điều phối; rollback bị chặn; huỷ đua với kết quả trễ; rò slot; commit status/sự kiện thiếu `cancelled`; log trực tiếp có thể mất | High | Accept | Phase 5, 6, 9, 11 |
| 12 | Dọn Docker: khoá GC BuildKit sai, danh sách giữ lại thiếu, không có cooldown | High | Accept | Phase 5 |
| 13 | Preview gọi production qua Traefik bằng `Host` giả; preview cùng site với control plane | Medium | Accept | Phase 1, 11 |
| 14 | Snapshot giải lại biến project khi rollback; watch paths thiếu kiểm tổ tiên, giới hạn 300 file, file đổi tên | Medium | Accept | Phase 5, 6, 8 |
| 15 | Dependencies phase 11, tham chiếu cũ, effort phase 12, watch paths cho stack; bỏ tuỳ chọn không được yêu cầu (`required_label`, giờ dọn/% cấu hình được, UI "cần redeploy", gỡ theo tuổi, `*_recovered`, `preview.rejected`) | Medium | Accept | Phase 4, 5, 8, 9, 11, 12, plan.md |

### Whole-Plan Consistency Sweep (bổ sung Dokploy)
- Files reread: plan.md, phase-01 … phase-12.
- Decision deltas checked: 12 (`deployments.preview_id`; tác giả + sender; buildkitd preview; zone theo (project, cụm); `tls_mode=preview_zone`; `cancelling`; bộ điều phối build; lease backup; vai trò token tính theo request; `reauth` theo phase; snapshot lưu giá trị đã giải; E2E thành phase 12).
- Reconciled stale references: 7 (tham chiếu "phase 11" ở phase 3, 7, plan.md; migration `00006_` ở phase 6; danh sách sự kiện phase 5; quy tắc thông báo phase 8; watch paths stack chuyển sang phase 9).
- Unresolved contradictions: 0. Còn tồn tại từ trước: `migrations/00010_invites_tunnels.sql` ở phase 10 chưa theo quy ước timestamp.

## Validation Log

### Session 1 — 2026-11-26
**Trigger:** bước validate của `/ak-plan --hard` sau red-team.
**Questions asked:** 7

#### Verification Results
- **Tier:** Full (11 phase)
- **Claims checked:** 46 (đối chiếu plan với hai báo cáo research; red-team đã kiểm thêm 28 claim)
- **Verified:** 43 | **Failed:** 0 | **Unverified:** 3
- Unverified còn lại (đều có test lab/thật trong phase sở hữu): `default-network-opts` áp cho overlay (phase 3), URL form tạo App trên GHES (phase 6), clone Forgejo bằng OAuth token (phase 6).

#### Questions & Answers

1. **[Scope]** v1 có hỗ trợ GitHub Enterprise Server không?
   - Options: Bỏ GHES khỏi v1 (Recommended) | Giữ GHES trong v1
   - **Answer:** Giữ GHES trong v1
   - **Rationale:** thêm base URL + test trên GHES trial; phase 6 tăng lên 3 tuần.
2. **[Scope]** Forgejo được hỗ trợ ở mức nào?
   - Options: Best-effort qua adapter Gitea (Recommended) | Hỗ trợ chính thức
   - **Answer:** Best-effort qua adapter Gitea
   - **Rationale:** tránh bộ test riêng; lệch API ghi vào known limitations.
3. **[Assumptions]** Ai làm (ước lượng 35 tuần)?
   - Options: 1 người full-time | 1 người part-time | 2 người trở lên
   - **Answer:** 1 người full-time
   - **Rationale:** phase chạy tuần tự; không cần quy tắc sở hữu file khi song song.
4. **[Tradeoffs]** Kích hoạt GC registry thế nào?
   - Options: Theo lịch + bấm tay (Recommended) | Chỉ bấm tay
   - **Answer:** Theo lịch + bấm tay
   - **Rationale:** dung lượng registry không phình khi người dùng quên.
5. **[Scope]** Thư viện template v1?
   - Options: Bộ 10 template phổ biến (Recommended) | Bộ nhỏ 5 template | Chỉ n8n + hướng dẫn đóng góp
   - **Answer:** Bộ 10 template phổ biến
   - **Rationale:** mỗi template phải chạy không cần `privileged`.
6. **[Architecture]** Chấp nhận Traefik rolling restart khi project mới có domain đầu tiên trong cụm (để giữ cách ly)?
   - Options: Chấp nhận (Recommended) | Dùng một network ingress chung
   - **Answer:** Chấp nhận
   - **Rationale:** giữ project là ranh giới cách ly thật.
7. **[Architecture]** Tên sản phẩm / module / repo?
   - Options: Tạm dùng deploy-service | Tôi sẽ nhập tên
   - **Answer:** Other
   - **Custom input:** "tự nghĩ, độc đáo chút"
   - **Rationale:** chọn **Bennghe**, prefix `bn`.

#### Confirmed Decisions
- GHES: giữ trong v1 — người dùng chọn, chấp nhận thêm thời gian phase 6.
- Forgejo: best-effort — không chặn phát hành.
- Nhân lực: 1 người full-time — ~36 tuần.
- Registry GC: lịch hằng tuần + nút chạy ngay.
- Template: bộ 10.
- Traefik gắn network theo project: chấp nhận restart.
- Tên: Bennghe.

#### Action Items
- [ ] Kiểm tra tên Bennghe còn trống (GitHub org/repo, domain, GHCR) trước phase 12.
- [ ] Đăng ký GHES trial cho test phase 6.

#### Impact on Phases
- Phase 4: rủi ro restart Traefik chuyển thành đánh đổi đã chấp nhận.
- Phase 5: cách kích hoạt GC.
- Phase 6: GHES trong phạm vi, Forgejo best-effort, effort 3 tuần.
- Phase 9: danh sách 10 template.
- Tất cả phase: prefix `ds` → `bn`, tên đường dẫn `deploy-service` → `bennghe`.

### Whole-Plan Consistency Sweep (Validation)
- Files reread: plan.md, phase-01 … phase-11.
- Decision deltas checked: 7.
- Reconciled stale references: 6 (GHES "chốt ở validate" phase 6; Forgejo test phase 6; GC phase 5; template phase 9; rủi ro Traefik phase 4; prefix/tên ở mọi phase).
- Unresolved contradictions: 0.

## Open Questions

- Phần bổ sung từ Dokploy (2026-11-26) đã qua red-team nhưng **chưa chạy bước validate** (người dùng yêu cầu commit ngay); chạy `/ak-plan validate` trước khi cook phase 11.
- `[UNVERIFIED]` mới: quyền App GitHub cho endpoint collaborator permission; endpoint permission/compare trên Gitea bản thấp nhất; người đẩy commit trong payload GitLab/Gitea; Traefik `ClientIP` thấy IP overlay của task preview.

Không còn câu hỏi chặn. Ba điểm `[UNVERIFIED]` được kiểm trong phase sở hữu.

<!-- slug: self-hosted-paas-v1 -->
