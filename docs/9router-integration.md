# router integration (import format)

router là Next.js dashboard + gateway, DB SQLite tại `/home/ubuntu/.router/db/data.sqlite`, bảng `provider_conns`.

## router đã hỗ trợ ZCode sẵn

- Provider `glm` (`oss/glm.js`): category `oauth`, dual-auth `["oauth","apikey"]`
- Khi account là **Start Plan**, executor `Executor` (`oss/glm.js`) tự route sang `zcode.z.ai`:
  - `Authorization: Bearer <zcodeJwtToken>` (không phải API key)
  - `X-Device-Mid: deviceMid()` (§81, tra từ `~/.router/zcode-device-mid` hoặc sinh mới)
  - `baseUrl: "https://zcode.z.ai/api/v1/zcode-plan/anthropic/v1/messages"`
- Token `zcodeJwtToken` nằm trong `providerSpecificData` của connection row: `provider=glm`, `authType=oauth`

→ **Không cần mod router.** Project claimer chỉ cần xuất JWT đúng schema, router tự tiêu thụ.

## Schema export (provider_conns row)

```json
{
  "id": "<uuid v4 — tự gen mỗi lần export>",
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

| Field | Bắt buộc | Ghi chú |
|---|---|---|
| `id` | Có | UUID v4, mỗi lần export gen mới (không tái dùng) |
| `provider` | Có | Phải là `"glm"` |
| `authType` | Có | Phải là `"oauth"` |
| `name` | Không | Hiển thị dashboard |
| `email` | Không | |
| `priority` | Không | Số nhỏ = ưu tiên cao (100 mặc định) |
| `isActive` | Không | `true` = tham gia rotation |
| `data.providerSpecificData.zcodeJwtToken` | **Bắt buộc** | Bearer cho Start Plan |
| `data.providerSpecificData.userId` | Nên có | |
| `data.providerSpecificData.username` | Nên có | |
| `data.providerSpecificData.zaiBusinessToken` | Nên có | |
| `data.testStatus` | Không | `"active"` sau khi claim OK |

## Import methods

**Option A: HTTP API (khuyên dùng)**
```bash
curl -X POST http://localhost:3000/api/providers \
  -H "Content-Type: application/json" \
  -d @export/zcode-export.json
```
Route: `src/app/api/providers/route.js` → `createProviderConnection` trong `src/lib/db/repos/connectionsRepo.js`

**Option B: Ghi trực tiếp SQLite**
```sql
INSERT INTO provider_conns(id, provider, authType, name, email, priority, isActive, data, createdAt, updatedAt)
VALUES('<uuid>', 'glm', 'oauth', 'ZCode Daily Claim', 'user@example.com', 100, 1, '<json-data>', strftime('%s','now'), strftime('%s','now'));
```
`data` là JSON string của phần `data` object (không bao gồm id/provider/...).

**Resolve X-Device-Mid của router:** `deviceMid()` trong executor đọc file `~/.router/zcode-device-mid` (UUID cố định theo máy). Claim ở Windows gen UUID riêng, nhưng router sẽ dùng UUID của chính nó khi inference — hai UUID không cần khớp nhau?? Thực tế cần **khớp** để upstream không ngửi ra "2 device" → hãy ghi UUID đã gen ở claimer vào file `~/.router/zcode-device-mid` trên máy chủ router.

## Sau import, test nhanh

```bash
curl http://localhost:3000/api/providers          # GET list, xác nhận connection mới
curl http://localhost:3000/api/v1/chat/completions -H "Content-Type: application/json" \
  -d '{"model":"glm-5.3","messages":[{"role":"user","content":"hi"}]}'
```

## router architecture (tham khảo)

```
src/app/api/v1/*
  → chatCore (oss/handlers/core.js)
  → Executor (default.js hoặc glm.js)
  → Provider upstream (api.z.ai hoặc zcode.z.ai)
  → Translator về chuẩn OpenAI
```

Thêm provider mới: `oss/providers/registry/{id}.js` (copy `REGISTRY_TEMPLATE.js`), đăng ký models vào `config/providerModels.js`, chỉ cần executor riêng khi không OpenAI-compatible. ZCode đã có sẵn, không cần làm mới.
