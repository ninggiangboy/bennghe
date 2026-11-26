---
phase: 9
title: "Compose & Templates"
status: pending
priority: P2
effort: "2w"
dependencies: [6, 7]
---

# Phase 9: Compose & Templates

## Goal
Deploy stack từ docker-compose (dán vào hoặc lấy từ Git) lên Swarm sau một bước biến đổi **dùng chung quy tắc của `servicespec`** (kiểm duyệt, route, secret, network, MTU, ghim volume), có cập nhật tuần tự và rollback theo revision; thư viện template cài bằng một thao tác.

## Requirements
- Parse bằng `compose-spec/compose-go` v2.
- **Biến đổi** (gọi helper của `servicespec`, không tự sinh quy tắc riêng): <!-- Red Team: N, E, F -->
  - Chạy chính sách kiểm duyệt của phase 5 trên từng service (bind mount, `cap_add`, `privileged`, host network, volume `external`, placement); vi phạm thì từ chối, trừ khi admin bật `privileged` cho stack.
  - **Xoá mọi label `traefik.*` người dùng viết**; route lấy từ bảng `domains` (owner_type `stack_service`) và được agent ingress sinh như app.
  - Service có `build:` thì build qua pipeline phase 5 rồi thay bằng `image@sha256:...` trong registry cụm.
  - Gắn `bn-proj-<id>`; đặt MTU tường minh cho mọi network khai báo và network `default` của stack (phòng khi `default-network-opts` không áp). <!-- Red Team: I -->
  - **Service mount named volume** được ghim `node.labels.bn.node==<id>` (node chọn ở lần deploy đầu, lưu cùng stack) và đặt `update_config.order=stop-first`; UI cảnh báo service có trạng thái không HA. <!-- Red Team: E -->
  - Env/secret từ UI đưa vào dạng Swarm secret tham chiếu; `stack_revisions` lưu bản có placeholder, không bao giờ lưu giá trị secret. <!-- Red Team: B, M -->
  - Thêm constraint `bn.ready`; trường Swarm bỏ qua (`container_name`, `depends_on` có điều kiện...) thì cảnh báo.
- **Deploy:** agent leader chạy `docker stack deploy --with-registry-auth --prune -c <file> <stack>` với file đã biến đổi, rồi xoá file khỏi đĩa ngay sau đó. Các lần deploy của cùng một stack chạy tuần tự như app (phase 5). <!-- Red Team: A -->
- **Revision:** mỗi lần deploy lưu một `stack_revisions`; rollback = deploy lại revision trước.
- **Stack từ Git:** dùng kết nối provider phase 6; webhook do `internal/cp/webhooks` quản lý với owner là stack; xoá stack hoặc đổi repo thì gỡ webhook. <!-- Red Team: N -->
- **Template:** `templates/<name>/compose.yaml` + `template.yaml` (tên, mô tả, icon, biến có generator: password, domain, email), nhúng vào binary. Bộ v1 gồm 10 template: n8n, WordPress+MySQL, Plausible, Uptime Kuma, Gitea, MinIO, Metabase, Ghost, Umami, Vaultwarden. <!-- Updated: Validation Session 1 - bộ 10 template --> Mỗi template phải qua biến đổi mà không cần `privileged`.
- **RBAC/audit/sự kiện:** route stack, revision, rollback, template có `x-permission` + audit; phát `deploy.*` cho stack.
- UI: tạo stack, xem cảnh báo biến đổi, danh sách service, log; thư viện template có form biến.

## Files to Create / Modify
- Create: `internal/cp/compose/` (parse, transform gọi `servicespec`), `internal/cp/stacks/`, `internal/cp/templates/`, `templates/n8n/`, `templates/<khác>/`
- Create: `internal/agent/stacks/`
- Create: `migrations/<timestamp>_stacks_templates.sql`
- Modify: `internal/cp/servicespec/`, `internal/cp/domains/` (owner `stack_service`), `internal/cp/webhooks/`, `internal/cp/deploy/` (tuần tự hoá cho stack), `proto/agent/v1/agent.proto`, `api/openapi.yaml`, `internal/cp/rbac/policy.go`, `web/src/routes/stacks/*`, `web/src/routes/templates/*` <!-- Red Team: N -->

## Tasks & Steps
1. Parse + transform qua helper `servicespec` (golden test).
2. Build service có `build:`.
3. Ghim volume + stop-first; secret placeholder.
4. Agent stack deploy + xoá file; tuần tự hoá.
5. Revision + rollback; stack từ Git + webhook.
6. Template loader + bộ đầu tiên.
7. RBAC/audit; UI.

## Todo
- [ ] Transform qua servicespec (admission, route, network, MTU)
- [ ] Build service có build:
- [ ] Ghim volume + stop-first, secret placeholder
- [ ] Agent stack deploy + xoá file + tuần tự hoá
- [ ] Revision + rollback, stack từ Git + webhook
- [ ] Template loader + bộ đầu tiên
- [ ] RBAC/audit cho route mới
- [ ] UI

## Success Criteria
- Cả 10 template cài được không cần `privileged` và truy cập HTTPS được.
- Template n8n có domain: HTTPS được; dữ liệu còn sau khi redeploy **và sau khi drain node rồi đưa node trở lại** (service ghim không bị dời sang volume rỗng).
- Compose có `build:` từ Git deploy và redeploy khi push.
- Compose có label `traefik.*` tự viết, bind mount `docker.sock` hoặc `container_name`: label bị xoá, bind mount bị từ chối, `container_name` có cảnh báo.
- Tải 50 MB giữa hai service của stack nằm trên hai node khác nhau không treo.
- `stack_revisions` trong Postgres không chứa giá trị secret.
- Rollback stack về revision trước trong một thao tác.

## Verification
- Golden test transform; lab Incus: cài từng template, drain node.

## Risks
- Nhiều compose cộng đồng cần quyền bị chính sách chặn. Phản ứng: thông báo rõ field nào bị chặn; admin có thể bật `privileged`.
