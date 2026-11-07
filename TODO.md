# TODO

## Đã xong (kiểm chứng trên Linux, box này)

- [x] Rust backend: build.rs, icons (32/128 PNG, ICO, ICNS), persistence.rs
  (SavedConfig + ClaimHistory), claim.rs (preview/claim/retry_action),
  balance.rs, captcha_config.rs, activation.rs
- [x] Frontend Vite: vite.config.js, `pnpm run build` pass, dist/ có
  index.html + ui/aliyun-captcha.js
- [x] ui/aliyun-captcha.js dịch hoàn toàn sang tiếng Việt; xóa
  ui/zcode-captcha-pool.js (ngoài phạm vi)
- [x] Frontend dùng `window.wbAliyunCaptcha.solve()`, persistence Rust
  (`load_saved_config`/`save_saved_config`), retry UI qua `#retryHint`
- [x] Cross-check bản tham chiếu/router → `docs/crosscheck.md`
- [x] `scripts/verify-export.py` với JWT giả 3 phần — pass
- [x] Grep CJK toàn project = 0 dòng; không lộ secret thật
- [x] README/TODO/docs/release-checklist.md cập nhật; commit + push

## P0 — chạy được claim thật trên Windows (Windows-only)

- [ ] `pnpm tauri dev` — test slider thật trên Windows (cần Node 20+,
  Rust stable, WebView2)
- [ ] Login ZCode lấy JWT (thử thủ công trước, ghi lại userId/username/email/zaiBusinessToken)
- [ ] Nhập JWT + X-Device-Mid vào UI → Preview gọi (xác nhận `plan_id` hôm nay xuất hiện)
- [ ] Kéo slider → Claim gọi 1 lần (check response 200 + `data.plan`)
- [ ] Export JSON → chạy `scripts/verify-export.py` → `POST /api/providers` vào router
- [ ] Chạy inference thử qua router (model `glm-5.3`) xem không bị 3007/3012
- [ ] Kiểm tra activation body rút gọn có bị upstream từ chối không
  (docs/crosscheck.md mục 4.2)

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

- Vào thư mục: `~/zcode-claimer/`
- Đọc theo thứ tự: `README.md` → `docs/architecture.md` → `docs/protocol.md` → `docs/integration.md` → `docs/security.md` → `TODO.md` (file này)
- Tham khảo (chỉ đọc, không sửa): `~/router/oss/`
- Giờ claim: 18:00 VN = 19:00 Bắc Kinh (mở bán); window dùng 22:00–08:00 VN
