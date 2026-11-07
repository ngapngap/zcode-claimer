# Architecture

```
User (Windows)
  └─ Tauri WebView ── @tauri-apps/api ── Rust backend (src-tauri/src/)
        │                                        │
        │ Aliyun SDK (o.alicdn.com)               │ reqwest
        │ solve() / mintTraceless()               ▼
        │                                  ZCode API (zcode.z.ai)
        │                                        │
        └─ verifyParam ── invoke("claim_plan") ──┘
```

## Luồng claim (slider path)

1. **Login ZCode** (web login, lấy JWT) — JWT lưu ở đâu xem `security.md`
2. **Activation** (`activation.rs`) — `POST /api/v1/client/activation` với `{event: "app_launch"}` rồi `{event: "app_daily_active"}`. Bắt buộc trước preview, thiếu → upstream trả `1004 ineligible`
3. **Preview** (`claim.rs::preview`) — `GET /api/v1/zcode-plan/billing/preview?app_version=3.14.0&platform=win32-x64` + headers `Authorization: Bearer {jwt}`, `X-Device-Mid: {uuid}`. Trả `plans[]`, lấy `plan_id` hôm nay (mỗi ngày 1 `plan_id` mới: `...-0928` → `...-0929`)
4. **Captcha config** (`captcha_config.rs`) — `GET /api/v1/client/configs?app_version=3.14.0&platform=win32-x64` → `data.configs.captcha {enabled, prefix, sceneId, region, skip_model_request}`. Nếu `enabled=false` → không bật slider. Nếu thiếu `skip_model_request` → mặc định cần token (an toàn)
5. **Slider** (`ui/aliyun-captcha.js` `solve()`) — `initAliyunCaptcha({SceneId, mode:"popup", captchaVerifyCallback, getInstance})` → warmup 2.1s → `button.click()` invisible 1×1 → user kéo → callback trả `{sceneId,certifyId,deviceToken,data}` → `verifyParam`
6. **Claim** (`claim.rs::claim`) — `POST /api/v1/zcode-plan/billing/claim {"plan_id":"..."}` + headers `X-Aliyun-Captcha-Verify-Param`, `X-Device-Mid`, `X-ZCode-App-Version`, `X-Platform`. Response: `code==0` + `data.plan` → Claimed; ngược lại map `code` → `ClaimFailure` (1001–401, xem `protocol.md`)
7. **Export** (`main.rs::export_router_json`) — tạo JSON đúng schema `provider_conns` của router (xem `integration.md`)

## Traceless pool (background inference)

File `ui/zcode-captcha-pool.js` (copy nguyên từ bản gốc) — vòng đời pool:

- Poll `GET /api/zcode/captcha` mỗi 2.5s (`maxPerRound=1`, backoff 3–15s)
- Target 3 token, đẩy qua `window.workbuddyDesktop.pushZcodeCaptchaTokens` → Rust `push()`
- Token TTL ~95s, one-instance-one-mint, validate: length ≥200, atob JSON, `securityToken` ≥50
- Nếu router không dùng pool này (đã có executor próprio), có thể tắt

## Error handling

| code | handling |
|---|---|
| 1003 AlreadyClaimed | Coi như thành công (đã lãnh hôm nay) |
| 1005 QuotaExhausted | Đợi `failure_ends_at` (upsteam cho window) |
| 3007 Captcha | Tắt frontend, làm lại slider |
| 401 LoginRequired | Login lại |
| 404 NotDeployed | Hoạt động chưa ra mắt, poll lại nhẹ |

## Nguồn code

- `claim.rs` (838 dòng bản gốc → port tối thiểu, bỏ plugin_host/routing/tracing)
- `captcha_config.rs` ← cùng file (phần `parse_captcha_config`)
- `activation.rs` (gọn lại 2 events)
- `aliyun-captcha.js` + `zcode-captcha-pool.js` — copy verbatim 773+253 dòng
