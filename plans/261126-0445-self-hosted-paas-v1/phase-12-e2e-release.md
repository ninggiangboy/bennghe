---
phase: 12
title: "E2E & Release"
status: pending
priority: P1
effort: "3.5w"
dependencies: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
---

# Phase 12: E2E & Release

## Goal
Bộ test end-to-end chứng minh mọi tiêu chí Done của plan, pipeline phát hành có ký số, tài liệu cài đặt/nâng cấp/khôi phục, và bản v1.0.

## Requirements
- **E2E:** `test/e2e/` (crate `bn-e2e`, `cargo nextest run -p bn-e2e --features e2e`) là test Rust gọi API control plane, chạy trên lab Incus (dựng từ phase 1); job nightly trên GitHub Actions dùng 3 VPS thật thuộc 2 nhà cung cấp, tạo và huỷ bằng `test/nightly/provision.sh` (CLI của nhà cung cấp, token trong secret của repo). Mỗi kịch bản ứng với một tiêu chí Done trong plan.md, cộng các kịch bản hỏng hóc từ red-team: <!-- Red Team: L, N -->
  - restore control plane từ backup cũ (agent vào chế độ nhận lại, không mất peer/service);
  - reboot manager khi control plane offline;
  - tắt đồng thời build node và một node app;
  - hai push liên tiếp vào cùng app;
  - agent N-2 nâng cấp qua kênh chỉ-cập-nhật;
  - PR của collaborator tạo preview, PR từ fork của người không có quyền ghi bị từ chối, commit do tài khoản ngoài đẩy vào fork của collaborator bị từ chối, production lỗi sau một preview `healthy` thì rollback về snapshot production, đóng PR gỡ preview (phase 11); nightly cần thêm một tài khoản không có quyền ghi trên mỗi provider; <!-- Updated: Dokploy comparison 2026-11-26 --> <!-- Red Team (Dokploy): 15 -->
  - CI dùng API token deploy một app; token thiếu scope bị 403 (phase 1).
- **Phát hành:** workflow GitHub Actions build control plane + agent + healthcheck + envexec cho `x86_64` và `aarch64` (target `*-unknown-linux-musl`, `cargo-zigbuild`), cùng profile release đã chốt ở spike phase 1; image control plane và ops image lên GHCR; ký minisign (đã chốt ở phase 2) và đăng checksum + chữ ký + `install-agent.sh` lên GitHub Releases; CHANGELOG. <!-- Red Team: K -->
- **Nâng cấp control plane:** `deploy/upgrade-controlplane.sh` chạy pg_dump trước khi kéo image mới và trước khi `sqlx migrate` chạy migration; tài liệu rollback = dừng bản mới, restore dump, chạy lại bản cũ (đổi epoch theo phase 2). Tài liệu ghi rõ không được nhảy quá một minor version nếu chưa qua kênh chỉ-cập-nhật. <!-- Red Team: L -->
- **Tương thích:** test agent bản N-1 kết nối control plane bản N; migration chỉ đi tiến.
- **Tài liệu** (`docs/`): cài đặt, thêm node, mô hình bảo mật (agent = root trên node, control plane bị chiếm thì node bị chiếm), backup control plane (pg_dump + master key), recovery kit, runbook khôi phục Swarm mất quorum, giới hạn đã biết (database không HA, cron không chạy bù, node sau NAT hai phía, preview chỉ cho app (không cho stack, không sao chép database), cảnh báo CPU/RAM cần control plane online).
- **README:** điểm khác biệt so với Coolify/Dokploy: agent kết nối ra (không SSH key, chạy sau NAT), đa cụm, agent nhẹ (≤ 10 MB RSS khi rảnh, đo trong CI), cron/backup chạy khi control plane offline.

## Files to Create / Modify
- Create: `test/e2e/*`, `test/nightly/provision.sh`, `test/nightly/teardown.sh`, `.github/workflows/nightly-e2e.yml`, `.github/workflows/release.yml`, `.cargo/config.toml`, `deploy/upgrade-controlplane.sh`
- Create: `docs/installation.md`, `docs/upgrade.md`, `docs/security-model.md` (agent = root trên node; project là ranh giới cách ly; chính sách kiểm duyệt; khoá control plane; 2FA, xác thực lại và API token, lệnh `controlplane admin reset-2fa` khi owner mất 2FA; bật preview nghĩa là tin mọi người có quyền ghi vào repo), `docs/backup-and-recovery.md` (kể cả restore control plane và chế độ nhận lại), `docs/runbooks/swarm-quorum-loss.md`, `docs/known-limitations.md`
- Modify: `README.md`

## Tasks & Steps
1. E2E harness trên lab có sẵn từ phase 1.
2. Viết kịch bản e2e cho từng tiêu chí Done.
3. Nightly trên VPS thật.
4. Build musl hai kiến trúc + ký minisign + GHCR + GitHub Releases; script nâng cấp có pg_dump.
5. Test tương thích N-1.
6. Tài liệu + README.
7. Gắn tag v1.0.0.

## Todo
- [ ] Harness e2e
- [ ] Kịch bản e2e đủ tiêu chí Done
- [ ] Nightly VPS thật
- [ ] Release pipeline có ký số
- [ ] Test tương thích N-1
- [ ] Tài liệu + README
- [ ] Tag v1.0.0

## Success Criteria
- Mọi kịch bản e2e xanh trên lab và trên một lần nightly thật.
- Agent bản phát hành rảnh ≤ 10 MB RSS trên VM 512 MB, đo trong nightly cho cả x86_64 và aarch64 (cam kết "agent nhẹ" trong README dựa vào con số này).
- Cài bản phát hành từ đầu theo `docs/installation.md` trên VM sạch thành công mà không cần bước ngoài tài liệu.

## Verification
- `cargo nextest run -p bn-e2e --features e2e`
- Kiểm chữ ký: thay 1 byte binary agent thì self-update từ chối; đẩy version thấp hơn thì agent từ chối.

## Risks
- Chi phí và độ chập chờn của nightly trên VPS thật. Phản ứng: chạy nightly thật mỗi tuần nếu chi phí cao; lab Incus là cổng bắt buộc cho mỗi PR lớn.
