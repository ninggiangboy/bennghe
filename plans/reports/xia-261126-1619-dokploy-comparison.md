# So sánh tính năng: Dokploy → Bennghe

## Nguồn

- Repo: `github.com/dokploy/dokploy`, nhánh mặc định, commit `6b02c8d541ec2f6d58994af01b556a8e9f4a8c55` (2026-11-25).
- Giấy phép: Apache-2.0, **trừ** các thư mục `/proprietary` (Dokploy Source Available License, cấm sao chép/phân phối). Phần proprietary gồm: audit log viewer, custom role, forward auth, SSO, SCIM, license key, whitelabeling. Báo cáo này không đọc mã của các phần đó.
- Stack khác hoàn toàn (TypeScript/Next.js/tRPC/Drizzle so với Go), nên thứ mang sang là **ý tưởng và hành vi**, không phải mã. Chế độ: `--compare`.

## Dự án cục bộ

Bennghe chưa có mã; nguồn sự thật là plan `plans/261126-0445-self-hosted-paas-v1/` (11 phase, scope HOLD). Vì vậy "mang vào" nghĩa là bổ sung yêu cầu vào phase phù hợp.

## Những chỗ Bennghe đã làm tốt hơn — không nên chép Dokploy

| Khía cạnh | Dokploy | Bennghe (plan) |
|---|---|---|
| Quản lý server | SSH từ panel (`ssh2`), panel giữ private key | Agent kết nối ra, desired state có chữ ký, không giữ SSH key |
| Hàng đợi | 3 hệ khác nhau: in-memory (self-host, mất khi restart), Inngest (cloud), BullMQ + Redis (schedules) | Một River queue trong Postgres, transaction-safe |
| Deploy image | Tag `<app>:v<n>` đẩy vào "rollback registry" riêng | Deploy theo digest + snapshot toàn bộ cấu hình |
| Backup | `docker exec pg_dump \| gzip`, upload và retention bằng rclone, giữ N file mới nhất | restic (mã hoá, dedup), credential chỉ-ghi tách credential prune, chạy được khi control plane offline |
| Webhook GitLab/Bitbucket | URL chứa `refreshToken` của tài nguyên, không có chữ ký | Kiểm chữ ký, khử trùng theo SHA, hỏi lại provider để chống replay |
| Traefik | Nhãn `traefik.*` người dùng tự viết được giữ lại khi update | Xoá mọi nhãn tự viết, route chỉ sinh từ bảng `domains` |
| Cách ly compose | Thêm hậu tố tên service/volume ("isolated deployment") | Network theo project + chính sách kiểm duyệt + namespace của stack |

Kết luận: phần lõi hạ tầng của plan không cần học thêm từ Dokploy. Giá trị nằm ở **các tiện ích phía người dùng** mà plan còn thiếu.

## Ứng viên nên mang vào

Ước lượng tính cho 1 người full-time, như plan. "Scope" cho biết có thay đổi phạm vi đã HOLD hay không.

### Nhóm A — nhỏ, nằm trong phạm vi hiện có, khuyến nghị thêm vào v1

| # | Tính năng | Dokploy làm thế nào | Cách làm cho Bennghe | Phase | Ước lượng |
|---|---|---|---|---|---|
| A1 | **Watch paths** cho monorepo | `application.watchPaths[]`, so khớp glob (micromatch) với file thay đổi trong payload push (`utils/watch-paths/should-deploy.ts`) | Thêm `watch_paths` cho app/stack. Lấy danh sách file qua compare API của provider (Bennghe đã gọi API để xác nhận SHA đầu branch). **Tránh lỗi của Dokploy:** khi payload thiếu danh sách file (GitLab chỉ gửi 20 commit), Dokploy trả về "không deploy"; Bennghe nên gọi compare API hoặc deploy cho an toàn | 6 | 2–3 ngày |
| A2 | **Huỷ deployment** | `cancelDeployment` + `killBuild` | Trạng thái `cancelled` trong state machine; huỷ được khi `queued/cloning/building/pushing`, không huỷ khi `rolling_out` (để Swarm tự xử lý) | 5 | 2 ngày |
| A3 | **Stream log build trực tiếp** | Một file log mỗi deploy, `tail -f` qua WebSocket | Plan chỉ ghi "build log nén trong Postgres". Nêu rõ: agent stream log build qua kênh WS hiện có, control plane fan-out tới UI, lưu bản nén khi xong | 5 | 2 ngày |
| A4 | **Dọn Docker trên node** | Cron `50 23 * * *` chạy prune image/container/build cache | Plan có GC registry nhưng chưa dọn image cũ, container đã dừng và cache BuildKit trên từng node. Trên VPS 1 GB/20 GB đĩa đây là nguyên nhân đầy đĩa phổ biến. Agent chạy prune theo lịch, giữ image đang chạy và image trong snapshot gần nhất | 5 hoặc 8 | 2–3 ngày |
| A5 | **Giới hạn build song song mỗi cụm** | `buildsConcurrency` theo server | Thêm cấu hình `max_concurrent_builds` (mặc định 1) cho build node; hàng đợi River theo cụm | 5 | 1 ngày |
| A6 | **Biến tham chiếu cấp project** | Env 3 tầng project/environment/service, cú pháp `${{project.X}}`, thiếu tham chiếu thì lỗi | Bảng `project_env` + cú pháp `${{project.X}}` trong env của app/stack, giải lúc render snapshot; tham chiếu tới secret vẫn đi đường Swarm secret | 5 | 3–4 ngày |
| A7 | **Backup volume nhất quán** | Dừng service → backup → khởi động lại → upload, giữ mã thoát | Plan ghi "không đảm bảo nhất quán". Thêm tuỳ chọn "dừng trong lúc backup" (scale về 0 → restic → scale lại), mặc định tắt, UI cảnh báo downtime | 7 | 2 ngày |
| A8 | **Thêm generator cho template** | `${password:N}`, `${base64:N}`, `${hash:N}`, `${uuid}`, `${timestamp}`, `${jwt}` | Plan có password/domain/email. Thêm base64, uuid, jwt (cần cho Supabase, Appwrite...) | 9 | 1 ngày |
| A9 | **Cảnh báo ngưỡng CPU/RAM** | Service Go riêng, so ngưỡng rồi POST callback | Plan đã có metrics 24 giờ và cảnh báo disk; thêm quy tắc CPU/RAM theo node, có thời gian giữ ngưỡng (ví dụ 5 phút) để không spam | 8 | 1–2 ngày |
| A10 | **Upload chứng chỉ tuỳ chỉnh** | `certificateType: custom` | Thêm `tls_mode=custom`; hữu ích nhất cho **Cloudflare Origin CA** khi bật proxy (Full strict) và cho cert nội bộ doanh nghiệp. Lưu mã hoá, cảnh báo hết hạn dùng chung cơ chế phase 4 | 4 | 2 ngày |

Tổng nhóm A ≈ 3,5–4,5 tuần nếu lấy hết, đẩy tổng plan từ ~36 lên ~40 tuần.

### Nhóm B — bảo mật tài khoản, khuyến nghị mạnh

Mô hình đe doạ của plan coi khoá ký của control plane là điểm đơn lẻ; nhưng **tài khoản owner/admin bị chiếm cũng tương đương** (admin bật được `privileged`, tải recovery kit). Plan chỉ có mật khẩu + rate limit.

| # | Tính năng | Dokploy | Cho Bennghe | Phase | Ước lượng |
|---|---|---|---|---|---|
| B1 | **TOTP 2FA** | better-auth plugin `twoFactor`, passkey | TOTP + mã khôi phục; owner có thể bắt buộc 2FA cho admin | 1 | 3–4 ngày |
| B2 | **Xác thực lại cho thao tác nhạy cảm** | Cửa sổ 5 phút sau khi nhập lại mật khẩu (`services/sensitive-action.ts`) | Plan đã có action riêng cho đọc nhạy cảm; thêm yêu cầu xác thực lại (mật khẩu hoặc TOTP) trong 5 phút cho: tải recovery kit, xem secret, bật `privileged`, mở public database, xoá cụm | 1 | 2 ngày |
| B3 | **API token cho CI** | Plugin `apiKey` + URL deploy theo tài nguyên | Plan chỉ có session cookie, nên CI (GitHub Actions build image rồi gọi deploy) không có cách gọi API. Thêm token cá nhân gắn vai trò + scope project, lưu hash, có hạn dùng, đi qua cùng middleware RBAC và audit. Không làm URL `refreshToken` không chữ ký như Dokploy | 1 hoặc 10 | 3 ngày |

### Nhóm C — giá trị cao nhưng **thay đổi phạm vi** (cần quyết định)

| # | Tính năng | Dokploy | Điều cần cân nhắc cho Bennghe | Ước lượng |
|---|---|---|---|---|
| C1 | **Preview deployment theo PR** | Host `preview-<app>-<6 ký tự>` trên wildcard, giới hạn số preview (mặc định 3), chỉ deploy PR có nhãn, kiểm quyền collaborator, xoá khi PR đóng | Tính năng được yêu thích nhất của Dokploy. Với Bennghe phải giải quyết: PR từ fork chạy mã tuỳ ý trên build node (cần chặn mặc định, chỉ collaborator); preview **không** được nhận secret production; domain preview phải nằm trong allowlist zone của project (cần wildcard DNS-01); giới hạn tài nguyên trên VPS nhỏ. Cần thêm sự kiện `pull_request` vào interface `Provider` | 2–3 tuần |
| C2 | **Tự động backup control plane** | Backup định kỳ thư mục + DB lên S3, có mã hoá | Plan chỉ có tài liệu pg_dump tay. Bennghe đã có restic, đích S3 và cơ chế epoch, nên job định kỳ backup Postgres của control plane (không kèm master key — master key nằm trong recovery kit) là phần mở rộng tự nhiên | 3–4 ngày |
| C3 | **Middleware theo domain**: redirect (www→apex, regex), basic auth, strip/add prefix | `utils/traefik/redirect.ts`, `security.ts`, `middleware.ts` | Phổ biến với người dùng. Phải mô hình hoá dạng có kiểu trong bảng `domains` để giữ nguyên tắc "không label Traefik tự viết"; agent ingress sinh middleware vào file config | 4–5 ngày |
| C4 | **Terminal vào container** | WebSocket `docker-container-terminal`, cả terminal host qua SSH | Hữu ích khi debug. Agent proxy `exec` qua kênh WS; đây là quyền rất mạnh: action riêng, xác thực lại (B2), audit mở/đóng phiên, không cho terminal host | 1 tuần |
| C5 | **Kiểm tra bảo mật node** | Đọc sshd (PermitRootLogin, PasswordAuthentication), ufw, fail2ban, gói chờ cập nhật | Agent là root nên báo cáo chỉ-đọc rất rẻ; hiển thị trên trang node. Không để panel tự sửa sshd | 3–4 ngày |
| C6 | Kênh thông báo Discord, ntfy | 12 kênh | Webhook HMAC đã phủ phần lớn; Discord và ntfy là hai kênh được self-hoster dùng nhiều nhất | 1–2 ngày |

### Không khuyến nghị

- **Environments (project → environment → service):** đổi mô hình RBAC/cách ly (scope `global|cluster|project`) và network. Người dùng có thể tạo project riêng cho staging/production; A6 đủ cho chia sẻ biến.
- **Nhiều builder** (Nixpacks, Heroku/Paketo buildpacks): Railpack + Dockerfile đã đủ và Nixpacks đang được Railway thay bằng Railpack.
- **Catalog template từ xa** (`templates.dokploy.com`): plan nhúng template vào binary để cài offline và kiểm qua chính sách kiểm duyệt; có thể thêm sau.
- **AI phân tích log, sandbox, vault provider, export telemetry, DNS provider khác, Bitbucket, GPU:** ngoài phạm vi hoặc là non-goal đã ghi.
- **Rollback registry, isolated compose bằng hậu tố tên:** Bennghe đã có cơ chế tốt hơn.
- Mọi thứ trong `/proprietary`.

## Rủi ro chung

- **Phạm vi:** plan đang HOLD 36 tuần cho 1 người. Lấy hết A + B + C sẽ thêm khoảng 10–12 tuần. Rủi ro chính là trễ alpha, không phải kỹ thuật.
- **Bảo mật:** C1 và C4 là hai tính năng mở rộng bề mặt tấn công nhiều nhất; nếu thêm, cần red-team lại phần liên quan.

## Khuyến nghị

1. Thêm **nhóm B** (≈ 1,5 tuần) vào phase 1 — nó lấp lỗ hổng thật trong mô hình đe doạ.
2. Thêm **A1–A5** (≈ 2 tuần) — rẻ, tránh sự cố vận hành (đầy đĩa, build đè nhau, deploy thừa trong monorepo).
3. A6–A10 và C2, C3: thêm nếu chấp nhận ~2,5 tuần nữa, hoặc đưa vào backlog v1.1.
4. **C1 (preview)** và **C4 (terminal)**: để v1.1, kèm red-team riêng.

## Quyết định của người dùng (2026-11-26)

Đưa vào plan v1: **nhóm B (B1–B3), A1–A10 và C1 (preview deployment)**. Các mục C2–C6 không được chọn, để v1.1. Ước lượng thêm ≈ 7,5–8,5 tuần trước buffer. C1 cần một vòng red-team riêng khi cập nhật plan.
