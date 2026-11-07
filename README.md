# zcode-claimer

Minimal Windows Tauri app — chỉ làm đúng 1 việc: **claim ZCode Start Plan hàng ngày** và **export JSON tương thích router**.

Tách ra từ `Agent2API` (fork), lột bỏ 95% phần thừa:

- ❌ Bỏ: `plugin_host`, 27 providers còn lại, Images API, routing groups, tracing, Agent page / mobile PWA / APK, deepseek-web, AI assistant
- ✅ Giữ: `zcode/claim.rs` (logic claim), `zcode/activation.rs`, `zcode/captcha.rs`, `ui/aliyun-captcha.js`, `ui/zcode-captcha-pool.js`

## Luồng

1. **Login ZCode** (JWT) — lấy `zcodeJwtToken` + `X-Device-Mid` ổn định
2. **Activation** `app_launch` / `app_daily_active` (báo active trước khi probe)
3. **Preview** `GET /api/v1/zcode-plan/billing/preview?app_version=3.14.0&platform=win32-x64` → lấy `plan_id` hôm nay
4. **Slider captcha** (Aliyun SDK trong Tauri WebView) → lấy `verifyParam`
5. **Claim** `POST /api/v1/zcode-plan/billing/claim {"plan_id":"..."}`
6. **Export** JSON đúng schema `provider_conns` của router → import vào `/api/providers`

## Giờ claim (giờ Việt Nam)

| Sự kiện | Giờ Bắc Kinh (UTC+8) | Giờ Việt Nam (UTC+7) |
|---|---|---|
| Mở claim | 19:00 | 18:00 |
| Dùng được | 23:00 – 09:00 hôm sau | 22:00 – 08:00 hôm sau |
| Ngày mới (AutoClaw-style) | 00:00 | 23:00 hôm trước |

## Export format (router)

```json
{
  "id": "<uuid>",
  "provider": "glm",
  "authType": "oauth",
  "name": "ZCode Daily Claim",
  "email": "user@example.com",
  "priority": 100,
  "isActive": true,
  "data": {
    "providerSpecificData": {
      "authMethod": "zcode-jwt",
      "username": "user@example.com",
      "userId": "<userId>",
      "zcodeJwtToken": "<JWT>",
      "zaiBusinessToken": "<token>"
    },
    "testStatus": "active"
  }
}
```

File này POST thẳng vào `POST /api/providers` của router (hoặc ghi trực tiếp vào `provider_conns` SQLite).

## Cấu trúc

```
zcode-claimer/
├── src-tauri/          # Rust backend tối thiểu (chỉ 3 commands)
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   └── src/main.rs     # claim + preview + captcha_config commands
├── src/                # Frontend 1 trang
│   ├── index.html
│   ├── main.js         # modal flow: login → preview → captcha → claim → export
│   └── styles.css
├── ui/
│   ├── aliyun-captcha.js       # copy từ Agent2API (773 dòng)
│   └── zcode-captcha-pool.js   # copy từ Agent2API (253 dòng)
├── docs/
│   └── protocol.md     # 2 endpoint + headers + error codes
└── export/             # output JSON ở đây
```

## Build (Windows)

```powershell
pnpm install
pnpm tauri dev
pnpm tauri build
```

Yêu cầu: Node 20+, Rust stable, WebView2 (Windows 10/11 có sẵn).

Không đụng vào Agent2API hay router — chỉ tham khảo protocol. router đã hỗ trợ ZCode sẵn qua provider `glm` (Start Plan executor), nên export là xong.
