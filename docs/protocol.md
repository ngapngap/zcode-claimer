# ZCode claim protocol (lấy từ Agent2API)

## 2 endpoint

```text
Preview: GET  https://zcode.z.ai/api/v1/zcode-plan/billing/preview?app_version=3.14.0&platform=win32-x64
Claim:   POST https://zcode.z.ai/api/v1/zcode-plan/billing/claim   body {"plan_id":"zcode-v3-start-plan-trust-0928"}
Config:  GET  https://zcode.z.ai/api/v1/client/configs?app_version=3.14.0&platform=win32-x64
         → data.configs.captcha { enabled, prefix, sceneId, region, skip_model_request }
```

## Headers bắt buộc

- `Authorization: Bearer <jwt>` (claim bắt buộc, preview cho anonymous)
- `X-Device-Mid: <uuid>` — **bắt buộc từ 2026-09-28**, thiếu → `400 {code:3001}`. PHẢI ổn định theo account, không gen lại mỗi lần
- `X-Aliyun-Captcha-Verify-Param: <verifyParam>` — chỉ gửi khi có giá trị (empty → 3007)
- `X-ZCode-App-Version: 3.14.0` (env `ZCODE_APP_VERSION` override được)
- `X-Platform: win32-x64`

## Error codes

| code | ý nghĩa |
|---|---|
| 1001 | plan không tồn tại |
| 1002 | không claim được (hết hạn/chưa bắt đầu) |
| 1003 | đã claim |
| 1004 | không đủ điều kiện (version/account) |
| 1005 | quota ngày hết |
| 3001 | parameter error (thường thiếu X-Device-Mid) |
| 3007 | captcha verify failed |
| 401 | chưa login / JWT hết hạn |

## Daily-once

Không có API "daily". Cách là mỗi ngày `plan_id` mới (`...-0928` → `...-0929`). Claim lại cùng plan_id → `1003`.

## Skip model request

`configs.captcha.skip_model_request=true` (trả về từ 2026-10-01) = inference không cần captcha token. Nếu thiếu field này → mặc định cần token (an toàn).

## Captcha

- Slider path: `initAliyunCaptcha({mode:"popup"})` → warmup 2.1s → click invisible button → user kéo → callback trả `{sceneId,certifyId,deviceToken,data}` → forward làm `verifyParam`
- Traceless path (mint cho background): `startTracelessVerification()` → base64 ~280 chars, TTL ~95s, one-instance-one-mint, pool target 3

Nguồn code: `desktop-tauri/src-tauri/server/src/server/core/providers/zcode/{claim.rs,activation.rs,captcha.rs}` trong Agent2API.

## Quota / balance (thêm sau)

```text
GET https://zcode.z.ai/api/v1/zcode-plan/billing/balance?app_version=3.14.0
Headers: Authorization Bearer {jwt}, X-Device-Mid {uuid}, User-Agent ZCode/3.14.0
Response: data { server_time, plans[], balances[] }
  plans[]: { plan_id, name, status, starts_at, ends_at, entitlements[].effective_at }
  balances[]: { bucket_id, show_name, meter, unit_type, total_units, used_units, reserved_units, remaining_units, available_units, period_start, period_end, expires_at }
```

UI quota hiển thị: tổng còn lại / đã dùng / tỉ lệ %, từng bucket (meter) + bar tiến độ, danh sách plan với tag (đang hiệu lực / chờ hiệu lực / đã hết hạn).

router dùng cùng endpoint này (`zstart.js::fetchStartBalance`, `parseStartBalance`, `balanceActivePlan`). router `startPlan.appVersion = "3.16.3"` (registry `glm.js`).
