# TODO (phiên mới — hoàn thiện)

## P0 — chạy được claim thật trên Windows

- [ ] `pnpm install` (cần Node 20+, Rust stable, WebView2)
- [ ] `pnpm tauri dev` — test slider thật trên Windows
- [ ] Login ZCode lấy JWT (thử thủ công trước, ghi lại userId/username/email/zaiBusinessToken)
- [ ] Nhập JWT + X-Device-Mid vào UI → Preview套餐 (xác nhận `plan_id` hôm nay xuất hiện)
- [ ] Kéo slider → Claim套餐 1 lần (check response 200 + `data.plan`)
- [ ] Export JSON → chạy `scripts/verify-export.py` → `POST /api/providers` vào router
- [ ] Chạy inference thử qua router (model `glm-5.3`) xem không bị 3007/3012

## P1 — auto-claim 18:00 VN mỗi ngày

- [ ] Cron / Tauri tray scheduler: 18:00 VN (= 19:00 Bắc Kinh) tự chạy preview + claim
- [ ] Nếu cần captcha mà không có người: báo user mở app kéo slider (không auto được — slider bắt buộc người thật)
- [ ] Thông báo khi claim OK (tray notification / log)

## P2 — robustness

- [ ] Retry logic: 1005 → đợi `failure_ends_at`; 3007 → làm lại captcha; 401 → login lại
- [ ] Lưu claim history (plan_id + ngày + kết quả) vào SQLite/file local
- [ ] Tra traceless pool: router executor có cần `X-Aliyun-Captcha-Verify-Param` cho inference không? Nếu có → cần mint pool riêng hoặc tắt pool guard
- [ ] Đổi version claimer khi upstream nâng `app_version` (env `ZCODE_APP_VERSION`)

## P3 — release

- [ ] `pnpm tauri build` → `.msi` / `.exe` cho Windows
- [ ] Icon app (thay placeholders trong `src-tauri/icons/`)
- [ ] README tiếng Việt + video demo

## Chuẩn bị phiên mới

- Vào thư mục: `/home/ubuntu/zcode-claimer/`
- Đọc theo thứ tự: `README.md` → `docs/architecture.md` → `docs/protocol.md` → `docs/integration.md` → `docs/security.md` → `TODO.md` (file này)
- Tham khảo (chỉ đọc, không sửa): `/home/ubuntu/Agent2API/desktop-tauri/.../providers/zcode/` và `/home/ubuntu/router/oss/`
- Giờ claim: 18:00 VN = 19:00 Bắc Kinh (mở bán); window dùng 22:00–08:00 VN
