# Release checklist (Windows-only)

Box Linux không có Rust toolchain và không phải Windows, nên các mục sau
**chưa kiểm chứng được tại đây** — phải chạy trên máy Windows 10/11.

## Release nhanh bằng script (khuyến nghị)

Trên máy Windows, tại thư mục gốc của repo:

```powershell
.\scripts\release.ps1 -Bump patch    # 0.1.11 -> 0.1.12
.\scripts\release.ps1 -Version 0.2.0 # version cụ thể
.\scripts\release.ps1 -Version 0.1.12 -DryRun  # chỉ xem các bước, không làm gì
```

Script tự làm: đổi version ở đúng 5 chỗ (package.json, src-tauri/Cargo.toml,
src-tauri/tauri.conf.json, src/index.html, src/main.js — mỗi chỗ kiểm tra đúng
1 lần khớp, sai thì dừng và hoàn tác) → chạy `pnpm run build` để kiểm tra
frontend (bỏ qua với `-SkipBuild`) → commit + push lên main → gắn tag
`v<version>` + push tag → workflow Build Windows x64 trên GitHub tự động build
.msi/.exe, tạo GitHub Release và cập nhật `latest.json` cho updater trong app.
Yêu cầu: đang ở branch main, cây git sạch, main đã pull mới nhất, tag chưa tồn tại.

## Môi trường cần có

- Node 20+ (khuyến nghị 20 LTS trở lên)
- Rust stable (cargo)
- WebView2 (Windows 10/11 có sẵn)
- pnpm (`corepack enable` hoặc cài qua npm)

## Các bước chạy

```powershell
pnpm install
pnpm run build        # chỉ build frontend (Vite) — đã kiểm chứng trên Linux
pnpm tauri dev        # mở app, test slider thật trên Windows
pnpm tauri build      # tạo .msi / .exe
```

## Kiểm chứng thủ công trên Windows

1. **Login**: nhập JWT thật + X-Device-Mid (UUID ổn định theo account),
   userId/username/email/zaiBusinessToken. Config được lưu qua Rust
   persistence trong app-data (không log JWT).
2. **Preview**: bấm "Báo activation + Preview gói" — xác nhận `plan_id`
   hôm nay xuất hiện. So sánh với bản gốc: activation body hiện rút gọn
   (chỉ `event`/`device_id`/`device_os_category`/`app_version`, thiếu
   `event_id`/`user_id`/`client_timezone`...). Nếu upstream từ chối,
   cần bổ sung trường tối thiểu theo `docs/crosscheck.md` mục 4.2.
3. **Captcha slider**: kéo slider thật trong WebView, xác nhận
   `verifyParam` được gửi kèm claim (thiếu header → 3007).
4. **Claim**: gọi 1 lần, check response 200 + `data.plan`. Kiểm tra các
   mã lỗi: 1005 (chờ tới `failure_ends_at`), 3007 (làm lại captcha),
   401 (login lại), 1003 (đã nhận).
5. **Balance/quota**: bấm Làm mới, xác nhận `total_remaining`/`used`/
   `granted`, plans, balances khớp dữ liệu thật. Lưu ý balance hiện
   thiếu query `platform` (router cũng chỉ gửi `app_version` — khả năng
   không bắt buộc, xem `docs/crosscheck.md` mục 4.1).
6. **Export**: tạo JSON, chạy `python scripts/verify-export.py`, sau đó
   `POST /api/providers` vào router. Chạy inference thử qua router
   (model `glm-5.3`) xem không bị 3007/3012.
7. **appVersion**: giữ `3.14.0` theo bản gốc (router dùng
   `3.16.3` mới hơn). Nếu upstream yêu cầu bản mới, chỉ cần đặt biến
   môi trường `ZCODE_APP_VERSION=3.16.3` khi chạy — không sửa code.
8. **Release**: `pnpm tauri build` → `.msi` / `.exe`. Icons đã có đủ
   (32/128 PNG, ICO, ICNS) trong `src-tauri/icons/`.

## Đã kiểm chứng trên Linux (box này)

- `pnpm install` + `pnpm run build` (Vite) — pass
- `node --check src/main.js` / `ui/aliyun-captcha.js` — pass
- `scripts/verify-export.py` với JWT giả 3 phần — pass (schema
  provider_conns: provider=glm, authType=oauth, zcodeJwtToken)
- Grep CJK toàn project (src/, src-tauri/, ui/, docs/, scripts/,
  README, TODO) = 0 dòng
- Không còn secret/domain thật nào trong code/docs (chỉ placeholder) — đã rà lại
  2026-11-07: dọn domain router, UUID demo, password mặc định khỏi docs/demo
  (lịch sử git cũ vẫn còn các giá trị đó, cần rewrite + force-push mới sạch hẳn)
