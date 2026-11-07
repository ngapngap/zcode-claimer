# POST /api/providers qua router ở máy chủ khác (2 máy khác nhau)

> Ví dụ máy chủ router: `https://router.example.com` (domain thật đã ẩn).
> Kết luận ngắn: **mặc định KHÔNG mở** — phải có dashboard auth (password → JWT cookie
> hoặc CLI token) hoặc tắt `requireLogin`. Bearer token / LLM API key của router
> **không** thay thế được. Xem chi tiết + ví dụ curl ở dưới.

## 1. Kết quả probe live trên `https://router.example.com`

| Request | Kết quả |
|---|---|
| `GET /api/health` | `200 {"ok":true}` (public) |
| `GET /api/auth/status` | `{"requireLogin":true,"authMode":"password",...}` → **requireLogin đang bật** |
| `GET /api/providers` (anonymous) | `401 {"error":"Unauthorized"}` |
| `POST /api/providers {}` (anonymous) | `401 {"error":"Unauthorized"}` (chặn ở guard, chưa tới validation) |

Kết luận: trên máy chủ này, không có auth thì không POST được.

## 2. Cơ chế auth của route (`guard.js`)

`/api/providers` nằm trong `PROTECTED_API_PATHS` và cơ chế `/api/*` là
**deny-by-default**. Request được qua khi có **1 trong 3**:

1. **Dashboard JWT cookie** `auth_token` (khuyên dùng cho máy khác):
   - `POST /api/auth/login` với `{"password":"<dashboard-password>"}` →
     server set cookie `auth_token` (HS256, hết hạn 24h).
   - Password mặc định là mật khẩu gốc của router nếu chưa đổi (`INITIAL_PASSWORD` hoặc hash trong DB),
     nhưng trên máy remote: nếu vẫn để mặc định và không có `INITIAL_PASSWORD`,
     server **từ chối cấp token** qua remote (chống CVE factory-default) → phải đổi
     password từ localhost trước, hoặc set `INITIAL_PASSWORD`.
   - Lưu ý tunnel: nếu `tunnelDashboardAccess !== true` thì login qua tunnel host bị
     `403 "Dashboard access via tunnel is disabled"`. Kiểm tra trong Settings →
     Profile / Endpoint trước.
2. **`x-9r-cli-token`** header (chỉ khi claimer chạy cùng máy với router):
   `sha256(machine-id + "9r-cli-auth" + DATA_DIR/auth/cli-secret)[0:16]`.
   Máy khác không biết secret này → không dùng được.
3. **`requireLogin === false`** trong Settings: khi đó `/api/providers` mở luôn,
   không cần auth. Mặc định là `true` (`src/lib/db/repos/settingsRepo.js:26`).
   Chỉ bật khi router nằm sau network riêng / không public — **không nên** bật
   cho máy public như `router.example.com`.

**Không liên quan đến Bearer token / LLM API key:** `Authorization: Bearer ...`,
`x-api-key`, `x-goog-api-key`, `?key=` chỉ dùng cho các prefix LLM public
(`/v1`, `/api/v1`, `/codex`, `/responses`) — guard `canAccessPublicLlmApi`.
Gửi LLM key vào `/api/providers` vẫn nhận `401`.

## 3. Network: cần gì khi 2 máy khác nhau

- router mặc định nghe **port 20128** (docker; dev là `20127`), không phải 3000.
  Ví dụ cũ trong `docs/integration.md` (`localhost:3000`) là sai port.
- Máy claimer không cần mở port gì. Máy chủ router phải cho claimer tới được:
  - qua tunnel (Cloudflare / Tailscale — `~/.router/tunnel/state.json` của máy chủ
    có `tunnelUrl`), hoặc
  - mở port 20128 ra internet + reverse proxy TLS (tránh expose port thô).
- Tauri backend gọi thẳng bằng reqwest → không vướng CORS. (Chỉ fetch từ browser
  sang origin khác mới cần CORS; `/api/providers` không có header CORS.)

## 4. Schema: JSON export của claimer KHÔNG post verbatim được

Route `POST /api/providers` (`src/app/api/providers/route.js:84-207`) đọc body
**phẳng**, không đọc `body.data.*` / `body.authType` / `body.isActive/server` :

```text
đọc:    provider, apiKey, name/displayName, priority, globalPriority,
        defaultModel, testStatus, providerSpecificData (TOP-LEVEL),
        proxyPoolId, id/allowOverwrite
bỏ qua: data.*, authType, isActive, email   ← phần claimer đang lồng vào data
```

Hai lỗi nếu post file export nguyên văn:

1. `providerSpecificData` sẽ thành `null` (route tìm ở top-level, không tìm trong `data`).
2. `400 "API Key is required"` — provider `glm` không có `apiKeyOptionalWith`
   nên body **bắt buộc** có `apiKey` non-empty (dòng 121-123). Kiểm tra: `glm` là
   `category:"oauth"` + `authModes:["oauth","apikey"]` → qua được `isValidProvider`,
   nhưng vẫn rớt ở check `apiKey`.

### Body đúng cho remote import (glm / Start Plan)

```json
{
  "provider": "glm",
  "apiKey": "<xem lưu ý>",
  "name": "ZCode Daily Claim",
  "priority": 100,
  "testStatus": "active",
  "providerSpecificData": {
    "authMethod": "zcode-jwt",
    "username": "<username>",
    "userId": "<userId>",
    "zcodeJwtToken": "<JWT>",
    "zaiBusinessToken": "<token>"
  }
}
```

Lưu ý `apiKey`: route luôn lưu row mới với `authType:"apikey"` (dòng 179) bất kể
gửi `authType` gì. Inference Start Plan chỉ cần `providerSpecificData.zcodeJwtToken`
(`core.js` set `zplan` khi có field này; `Executor` dùng
`Authorization: Bearer <zcodeJwtToken>`) → Start Plan vẫn chạy. Nhưng đường
paid-plan (api.z.ai transport) cần `apiKey`/`accessToken` thật (planApiKey do OAuth
mint ra) → gửi dummy/`zaiBusinessToken` chỉ đủ cho Start Plan. Để có đủ cả 2,
hãy dùng **OAuth login flow trên chính máy chủ** (`/api/oauth/[provider]/...`) hoặc
ghi thẳng SQLite với `authType="oauth"` + `accessToken=<planApiKey>`.
Trùng `name` sẽ bị `409 PROVIDER_NAME_CONFLICT` nếu không gửi `id` +
`allowOverwrite:true`.

### Ví dụ curl (máy claimer → `https://router.example.com`)

```bash
# 1) Lấy cookie auth (giữ cookie jar; cần đã bật tunnelDashboardAccess nếu qua tunnel)
curl -c cookies.txt -X POST https://router.example.com/api/auth/login \
  -H "Content-Type: application/json" -d '{"password":"<DASHBOARD_PASSWORD>"}'

# 2) POST provider (apiKey top-level + providerSpecificData top-level)
curl -b cookies.txt -X POST https://router.example.com/api/providers \
  -H "Content-Type: application/json" -d '{
    "provider":"glm",
    "apiKey":"<zaiBusinessToken-hoặc-planApiKey>",
    "name":"ZCode Daily Claim",
    "priority":100,
    "testStatus":"active",
    "providerSpecificData":{
      "authMethod":"zcode-jwt",
      "username":"<username>",
      "userId":"<userId>",
      "zcodeJwtToken":"<JWT>",
      "zaiBusinessToken":"<token>"
    }
  }'

# 3) Xác nhận
curl -b cookies.txt https://router.example.com/api/providers | head -c 400
```

Nếu `requireLogin === false` trên máy chủ, bỏ bước 1 (không cần cookie).

## 5. Danh sách kiểm tra trước khi chạy (2 máy khác nhau)

- [ ] Máy chủ router reachable từ máy claimer (tunnel URL hoặc port + TLS).
- [ ] `tunnelDashboardAccess === true` nếu đi qua tunnel (Settings), và password đã đổi khỏi mật khẩu mặc định.
- [ ] Có dashboard JWT cookie, hoặc `requireLogin === false`, hoặc claimer cùng máy (CLI token).
- [ ] Body phẳng: `apiKey` + `providerSpecificData` ở top-level (không lồng trong `data`).
- [ ] `X-Device-Mid`: ghi UUID claimer vào `~/.router/zcode-device-mid` **trên máy chủ**
      (router `deviceMid()` đọc file này; lệch UUID → upstream dễ 3012).
- [ ] Sau import: `POST /api/providers/[id]/test` hoặc chat thử
      `/api/v1/chat/completions` với LLM key của router để kiểm tra Start Plan route.

## Tham khảo

- Guard: `guard.js` (`PROTECTED_API_PATHS`,
  `isAuthenticated`, `ALWAYS_PROTECTED`, `canAccessPublicLlmApi`)
- Route: `~/router/src/app/api/providers/route.js:84-207`
- Session: `~/router/src/lib/auth/dashboardSession.js` (JWT 24h, password mặc định của router)
- CLI token: `~/router/src/shared/utils/machineId.js` (`getConsistentMachineId`)
- glm registry: `~/router/oss/glm.js`
  (dual-auth, không có `apiKeyOptionalWith`)
- Start Plan: `~/router/oss/handlers/core.js:130-154`,
  `~/router/oss/glm.js:70,253,287`
