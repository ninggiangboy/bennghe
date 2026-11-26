---
phase: 6
title: "Git Providers"
status: pending
priority: P1
effort: "3w"
dependencies: [5]
---

# Phase 6: Git Providers

## Goal
Tích hợp native GitHub (GitHub App), GitLab và Gitea/Forgejo: kết nối tài khoản, chọn repo từ danh sách, tự tạo webhook, push lên branch thì tự deploy, và cập nhật commit status theo kết quả deployment.

## Requirements
- Interface chung (ba biến thể thật):
  ```go
  type Provider interface {
      Kind() Kind
      ListRepos(ctx, conn) ([]Repo, error)
      ListBranches(ctx, conn, repo) ([]string, error)
      EnsureWebhook(ctx, conn, repo, url, secret) (hookID string, err error)
      DeleteWebhook(ctx, conn, repo, hookID) error
      ParseWebhook(r *http.Request, secret []byte) (PushEvent, error)
      CloneCredential(ctx, conn, repo) (CloneAuth, error) // ngắn hạn
      SetCommitStatus(ctx, conn, repo, sha string, s Status) error
  }
  ```
- **GitHub:** GitHub App Manifest flow: người dùng bấm "Tạo GitHub App", control plane POST manifest (webhook URL, quyền contents:read, metadata:read, statuses:write), nhận code, đổi lấy app id + private key + webhook secret + client secret, lưu mã hoá; sau đó cài App vào tài khoản/org. Clone bằng installation token. Kiểm `X-Hub-Signature-256`. Hỗ trợ GitHub Enterprise Server trong v1 (người dùng chốt ở Validation Session 1): <!-- Updated: Validation Session 1 - giữ GHES --> endpoint đổi code là `https://HOSTNAME/api/v3/app-manifests/{code}/conversions` (có tài liệu); URL form tạo App trên GHES (`https://HOSTNAME/settings/apps/new`) là suy luận [UNVERIFIED: cần test trên GHES thật].
- **GitLab:** OAuth2 application có PKCE (gitlab.com hoặc self-managed theo base URL), lưu access + refresh token mã hoá; access token sống 2 giờ và mỗi lần refresh đổi cả hai token, nên việc refresh phải nằm trong transaction có khoá dòng để hai worker không refresh đồng thời; project hook với secret token (`X-Gitlab-Token`); commit status API; clone bằng OAuth token.
- **Gitea/Forgejo:** Forgejo ở mức best-effort qua adapter Gitea (Validation Session 1): chạy chung bộ test Gitea trong job nightly mang tính tham khảo, không chặn phát hành khi lệch. <!-- Updated: Validation Session 1 - Forgejo best-effort --> OAuth2 application theo base URL; repo webhook với secret, kiểm HMAC-SHA256 qua `X-Gitea-Signature` (Forgejo gửi cả `X-Forgejo-Signature` lẫn `X-Gitea-Signature`, nên một adapter dùng chung; API OAuth/webhook/status chưa lệch nhau theo research-02 mục 3); clone bằng OAuth token trên Forgejo [UNVERIFIED: cần test tích hợp]; commit status API.
- Webhook endpoint `/webhooks/{provider}/{connection-id}`: <!-- Red Team: M -->
  - Kiểm chữ ký **đồng bộ trước khi** đưa vào hàng đợi; so sánh constant-time (kể cả `X-Gitlab-Token`); từ chối hook Gitea có secret rỗng.
  - Khử trùng theo `(connection, ref, after_sha)` ngoài delivery ID (delivery ID không nằm trong phần được ký nên không chống replay).
  - Trước khi deploy, hỏi API provider để xác nhận SHA vẫn là đầu branch; nếu không thì bỏ qua (chống replay payload cũ).
  - Trả 2xx nhanh rồi xử lý qua River; deployment đi qua hàng đợi tuần tự của app (phase 5).
- Push vào branch theo dõi thì tạo deployment; commit status `pending` khi bắt đầu, `success`/`failure` khi kết thúc, link về trang deployment.
- Webhook có owner là app hoặc stack (phase 9 dùng lại); đổi repo/branch hoặc xoá owner thì gỡ webhook cũ. <!-- Red Team: N -->
- **Luồng OAuth và Manifest:** `state` sinh ngẫu nhiên, gắn với session của người bắt đầu luồng, kiểm khi callback; base URL (GitLab self-managed, Gitea, GHES) đi qua `netpolicy`; lỗi từ provider không trả nguyên nội dung response về UI. <!-- Red Team: M -->
- **RBAC/audit:** kết nối provider là thao tác admin; chọn repo cho app theo quyền project; mọi route có `x-permission` + audit. <!-- Red Team: C -->

## Files to Create / Modify
- Create: `internal/cp/gitprovider/provider.go`, `internal/cp/gitprovider/github/`, `internal/cp/gitprovider/gitlab/`, `internal/cp/gitprovider/gitea/`, `internal/cp/webhooks/`
- Create: `migrations/00006_git_connections.sql`
- Modify: `internal/cp/deploy/` (trigger từ push, gọi SetCommitStatus), `api/openapi.yaml`, `internal/cp/rbac/policy.go`, `web/src/routes/settings/git*`, wizard tạo app

## Tasks & Steps
1. Interface + test contract chung (một bộ test chạy cho mọi provider bằng fixture).
2. GitHub App Manifest flow + installation + token + webhook + status.
3. GitLab OAuth + hook + status.
4. Gitea/Forgejo OAuth + hook + status.
5. Webhook endpoint + khử trùng + River.
6. Nối deploy pipeline với commit status.
7. UI kết nối provider, chọn repo/branch trong wizard.

## Todo
- [ ] Interface + contract test
- [ ] GitHub App Manifest flow
- [ ] GitLab
- [ ] Gitea/Forgejo
- [ ] Webhook endpoint + khử trùng
- [ ] Commit status
- [ ] UI

## Success Criteria
- Với mỗi provider (GitHub.com, GHES, GitLab, Gitea; Forgejo best-effort): kết nối, chọn repo, push thì app tự deploy và commit status chuyển `success`; push bản lỗi thì status `failure`.
- Webhook gửi lại cùng delivery ID không tạo deployment thứ hai.
- Webhook chữ ký sai trả 401; gửi lại payload cũ đã ký với delivery ID mới không tạo deployment.
- Callback OAuth/Manifest với `state` không khớp session bị từ chối.

## Verification
- Contract test bằng fixture trong CI.
- Test sống: Gitea và Forgejo container trong CI; GitLab CE container trong job nightly (nặng); GitHub bằng một org thử nghiệm, chạy tay hoặc nightly; GHES bằng bản dùng thử (trial) của GitHub, chạy tay trước khi đóng phase.

## Risks
- Control plane không có URL public thì webhook không tới được. Phản ứng: UI báo rõ, cho phép nút "Deploy" tay.
- API Forgejo lệch Gitea theo phiên bản. Tín hiệu: test Forgejo nightly đỏ. Phản ứng: ghi vào known limitations; sửa nếu chi phí nhỏ (best-effort).
- URL form tạo App trên GHES là suy luận. Tín hiệu: test GHES trial thất bại ở bước manifest. Phản ứng: hướng dẫn tạo App tay trên GHES rồi nhập app id + private key.
