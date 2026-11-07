# Security notes

## X-Device-Mid (cực kỳ quan trọng)

- **Phải ổn định theo account.** gen 1 lần (UUID v4) rồi lưu máy, tái dùng cho mọi request (`preview`, `claim`, `activation`). docs/protocol.md đã ghi: 2026-09-28 upstream bắt buộc header này, thiếu → `400 {code:3001}`.
- **Không gen lại mỗi lần.** Số UUID thay đổi = upstream thấy "1 account đổi device mỗi ngày" → risk control.
- **Khớp với router:** khi export, ghi UUID đã dùng claim vào file `~/.router/zcode-device-mid` trên máy chủ (router `deviceMid()` đọc file này). 2 UUID khác nhau (claimer vs router) = upstream tưởng 2 thiết bị → dễ bị 3012.

## JWT

- `zcodeJwtToken` là Bearer token cho Start Plan, expiry theo server (phải login lại khi hết hạn → `401`)
- Lưu ở đâu: file local của Tauri app (AppData) với quyền user, không commit vào git
- Đừng log JWT ra console / file log
- Trade-off của `localStorage`: bản dev trước đây lưu JWT trong `localStorage` (chỉ để tiện cho dev/mở bằng trình duyệt thuần). Cách này vẫn hoạt động nhưng JWT nằm trên đĩa dạng text, dễ bị đọc bởi bất kỳ script nào chạy trong WebView; **bản release phải chuyển sang Rust persistence** qua Tauri secure store (file trong `app-data`, quyền user — đã có `load_saved_config`/`save_saved_config` ở `src-tauri/src/persistence.rs`, UI gọi khi mở app và khi thay đổi). Khi dùng Tauri, frontend không dùng `localStorage` cho JWT nữa.

## Captcha verifyParam

- One-time, TTL ~2 phút (traceless) / theo slider session
- Không lưu, không replay. Mỗi claim lấy 1 lần
- `X-Aliyun-Captcha-Verify-Param` chỉ gửi khi có giá trị (rỗng → upstream 3007)

## File export

- Chứa `zcodeJwtToken` — **không commit `.gitignore` đã bỏ qua `export/*.json`**
- Che key khi chia sẻ (repo public, screenshot...)
- Xóa token khi không dùng nữa (đổi mật khẩu ZCode)

## Network

- Tauri CSP = `null` (cần để SDK Aliyun từ `o.alicdn.com` load). Nếu khóa CSP, phải allowlist `https://o.alicdn.com` + `https://*.alicdn.com` (SDK còn kéo thêm resource)
- `ZCODE_APP_VERSION` env override được (default `3.14.0`). Đừng dùng version lạ (sẽ bị `1004 ineligible`)
