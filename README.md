# zcode-claimer

Minimal Windows Tauri app — chỉ làm đúng 1 việc: **claim ZCode Start Plan hàng ngày** và **export JSON chứa token để dùng tiếp**.

Tách ra từ `Agent2API` (fork), lột bỏ 95% phần thừa:

- ❌ Bỏ: `plugin_host`, 27 providers còn lại, Images API, routing groups, tracing, Agent page / mobile PWA / APK, deepseek-web, AI assistant
- ✅ Giữ: `zcode/claim.rs` (logic claim), `zcode/activation.rs`, `zcode/captcha.rs`, `ui/aliyun-captcha.js` (đã dịch sang tiếng Việt)
- ❌ Bỏ tiếp: `ui/zcode-captcha-pool.js` (ngoài phạm vi 4 tính năng hiện tại)

## Luồng

1. **Login ZCode** (JWT) — lấy `zcodeJwtToken` + `X-Device-Mid` ổn định
2. **Activation** `app_launch` / `app_daily_active` (báo active trước khi probe)
3. **Preview** `GET /api/v1/zcode-plan/billing/preview?app_version=3.14.0&platform=win32-x64` → lấy `plan_id` hôm nay
4. **Slider captcha** (Aliyun SDK trong Tauri WebView) → lấy `verifyParam`
5. **Claim** `POST /api/v1/zcode-plan/billing/claim {"plan_id":"..."}`
6. **Export** JSON (`provider: glm`, `authType: oauth`) ra thư mục `export/`

## Giờ claim (giờ Việt Nam)

| Sự kiện | Giờ Bắc Kinh (UTC+8) | Giờ Việt Nam (UTC+7) |
|---|---|---|
| Mở claim | 19:00 | 18:00 |
| Dùng được | 23:00 – 09:00 hôm sau | 22:00 – 08:00 hôm sau |
| Ngày mới (AutoClaw-style) | 00:00 | 23:00 hôm trước |

> Lưu ý: scheduler 18:00 VN tự động là tính năng ngoài phạm vi hiện tại
> (UI thủ công: Login → Preview → Captcha → Claim → Export). Xem TODO P1.

## Export format

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

File JSON được ghi ra thư mục `export/`, chứa JWT và token ZCode.

## Cấu trúc

```
zcode-claimer/
├── src-tauri/          # Rust backend tối thiểu (Tauri commands)
│   ├── Cargo.toml
│   ├── build.rs        # xác nhận icons cho Tauri
│   ├── tauri.conf.json # frontendDist=../dist, devUrl 5173
│   ├── icons/          # 32x32/128x128 PNG, ICO, ICNS
│   └── src/
│       ├── main.rs             # đăng ký commands
│       ├── claim.rs            # preview + claim + retry_action
│       ├── activation.rs       # app_launch / app_daily_active
│       ├── balance.rs          # query_balance (quota/plans/buckets)
│       ├── captcha_config.rs   # GET /api/v1/client/configs
│       └── persistence.rs      # SavedConfig + ClaimHistory (app-data JSON)
├── src/                # Frontend Vite (root=src, build ra dist/)
│   ├── index.html      # load ui/aliyun-captcha.js 1 lần trước main.js
│   ├── main.js         # modal flow: login → preview → captcha → claim → export
│   └── styles.css
├── ui/
│   └── aliyun-captcha.js   # slider captcha (đã dịch sang tiếng Việt)
├── vite.config.js      # copy ui/aliyun-captcha.js vào dist/ui/ khi build
├── docs/               # protocol, architecture, security, release-checklist
├── scripts/
│   └── verify-export.py  # kiểm tra JSON export hợp lệ
└── export/             # output JSON ở đây
```

## Build (Windows)

```powershell
pnpm install
pnpm tauri dev
pnpm tauri build
```

Yêu cầu: Node 20+, Rust stable, WebView2 (Windows 10/11 có sẵn).

Không đụng vào Agent2API — chỉ tham khảo protocol.
