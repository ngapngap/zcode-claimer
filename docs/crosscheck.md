# Đối chiếu với bản tham chiếu và router

Ngày kiểm tra: 2026-11-07 (box Linux, không build Rust, chỉ đọc tĩnh).
Chỉ đọc thư mục tham khảo, không sửa.

- Bản tham chiếu: code ZCode gốc đã đối chiếu khi port
  (`claim.rs`, `activation.rs`, `captcha.rs`, `balance.rs`)
- router: `/home/ubuntu/router/oss/services/zstart.js`,
  `/home/ubuntu/router/oss/glm.js`,
  `/home/ubuntu/router/oss/glm.js`
- Bản của dự án: `src-tauri/src/claim.rs`, `src-tauri/src/activation.rs`,
  `src-tauri/src/balance.rs`, `src-tauri/src/captcha_config.rs`, `ui/` (giữ nguyên bản gốc)

Kết luận nhanh: protocol chính (endpoint, header, mã lỗi) khớp bản gốc.
Ba điểm lệch cần biết: `appVersion`, body activation rút gọn, balance thiếu `platform`
và `User-Agent` giống bản gốc. Chi tiết theo từng mục dưới đây.

## 1. Endpoint

| Chức năng | Bản gốc | Dự án | router (`glm.js` startPlan) |
|---|---|---|---|
| Preview | `GET /api/v1/zcode-plan/billing/preview?app_version=&platform=` | Khớp | Không có (router không claim) |
| Claim | `POST /api/v1/zcode-plan/billing/claim`, body `{"plan_id": "..."}` | Khớp | Không có |
| Balance | `GET /api/v1/zcode-plan/billing/balance?app_version=&platform=` | Khớp path, thiếu `platform` (xem mục 4) | `balanceUrl` cùng path, nhưng chỉ gắn `app_version` |
| Activation | `POST /api/v1/client/activation` | Khớp | Không có |
| Cấu hình captcha | `GET /api/v1/client/configs?app_version=&platform=` | Có trong `captcha_config.rs` | Không có |

Origin mặc định: `https://zcode.z.ai` (khớp cả ba bên).

## 2. Phiên bản app (`appVersion`)

- Bản tham chiếu: `DEFAULT_APP_VERSION = "3.14.0"` (`claim.rs:86`),
  cho phép ghi đè bằng biến môi trường `ZCODE_APP_VERSION`
  (hàm `app_version()`).
- Dự án: giữ `3.14.0` + ghi đè môi trường (khớp bản gốc).
- router: `appVersion: "3.16.3"` (`glm.js:46`).

Điểm lệch cần biết: router đang dùng `3.16.3` mới hơn. Nếu upstream bắt
phiên bản mới nhất thì `3.14.0` vẫn chạy được theo bản gốc hiện tại, nhưng
nên cân nhắc đồng bộ lên `3.16.3` qua biến môi trường khi test trên Windows.
Không có bằng chứng bản gốc đã bỏ `3.14.0` — đây là khác biệt cấu hình,
không phải khác biệt protocol.

## 3. Header

### Preview

| Header | Bản gốc | Dự án |
|---|---|---|
| `Authorization: Bearer <jwt>` | Chỉ gửi khi jwt không rỗng (anonymous probe được phép) | Khớp |
| `X-Device-Mid` | Gửi khi có giá trị; bắt buộc thực tế từ 2026-09-28, giá trị phải là UUID | Khớp |

### Claim (thứ tự giống bản gốc)

`Authorization` → `Content-Type` → captcha (nếu có) → `X-ZCode-App-Version`
→ `X-Platform` → `X-Device-Mid`. Cả hai đều chỉ gửi header captcha
`X-Aliyun-Captcha-Verify-Param` / `X-Aliyun-Captcha-Verify-Region` khi có
giá trị (tránh bị upstream coi là "captcha không hợp lệ" và trả 3007).

### Balance

- Bản gốc: `Authorization`, `Accept`, `X-Device-Mid` (khi có). Không gửi
  `User-Agent`.
- Dự án: thêm `User-Agent: ZCode/<appVersion>` — giống router hơn
  (`zstart.js` gửi `User-Agent` cho cả balance lẫn subscription).
  Thêm header này không gây hại nhưng là điểm khác bản gốc.
- router: gửi `X-Device-Mid` + `User-Agent: ZCode/<appVersion>`.

## 4. Điểm lệch đã xác nhận (cần sửa hoặc chấp nhận)

1. **Balance thiếu `platform`** — bản gốc gửi `?app_version=&platform=`
   (`balance.rs:269`), dự án chỉ gửi `?app_version=` (`balance.rs:71`).
   router cũng chỉ gửi `app_version`, nên khả năng cao không bắt buộc,
   nhưng để khớp bản gốc thì nên thêm `platform=win32-x64`.
2. **Activation body rút gọn** — bản gốc gửi body đầy đủ: `event_id` (UUID mới
   mỗi event), `client_timezone`, `client_language` (`zh-CN`),
   `element_name`, `event_region` (`app`), `event_type` (`view`),
   `device_os_version`, `device_mid`, `mac_id` (`""`),
   `marketing_params` (`"{}"`), `screen_resolution` (`2560x1440`),
   `user_id` (bắt buộc, thiếu thì lỗi). Dự án chỉ gửi
   `event`, `device_id`, `device_os_category`, `app_version` và nuốt lỗi
   (best-effort).
   Rủi ro: upstream có thể coi event thiếu trường là không hợp lệ, khi đó
   "báo activation trước preview" không có tác dụng. Khuyến nghị bổ sung
   trường tối thiểu khi sửa trên Windows.
3. **`appVersion 3.14.0` vs `3.16.3`** của router (mục 2).
4. **Không có Region/Proxy** — bản gốc truyền `Region` (chọn origin theo
   vùng) và `ResolvedProxy`; dự án hardcode `https://zcode.z.ai` và không
   hỗ trợ proxy. Với mục tiêu "claim trên Windows cá nhân" thì chấp nhận
   được, nhưng không tương đương đầy đủ bản gốc.
5. **Scheduling** — bản gốc có `outcome_hold`/`NextAttempt`
   (Claimed/AlreadyClaimed → HoldUntil, QuotaExhausted → WaitWindow,
   còn lại → Cooldown); dự án chỉ trả `retry_action` cho frontend. Logic
   tương đương về ý, chưa có bộ kiểm tra tự động.

## 5. Mã lỗi nghiệp vụ (khớp hoàn toàn)

| Mã | Ý nghĩa (bản gốc) | Dự án |
|---|---|---|
| 1001 | Không tìm thấy gói | `NotFound` |
| 1002 | Hoạt động kết thúc / chưa bắt đầu | `Unavailable` |
| 1003 | Đã nhận rồi | `AlreadyClaimed` → `done` |
| 1004 | Không đủ điều kiện | `Ineligible` |
| 1005 | Hạn mức trong ngày đã hết | `QuotaExhausted` → `wait_failure_ends_at` |
| 3001 | Tham số lỗi (thường do thiếu `X-Device-Mid`) | `InvalidRequest` |
| 3007 | Captcha không đạt | `Captcha` → `retry_captcha` |
| 401 | JWT hết hạn / chưa đăng nhập | `LoginRequired` → `relogin` |

Preview trả 404 → `NotDeployed`; claim cần JWT (jwt rỗng → 401 `LoginRequired`).
Cả hai khớp bản gốc.

## 6. Export cho router (`scripts/verify-export.py`)

Schema đúng như `docs/integration.md` và `export/README.md`:
`provider = "glm"`, `authType = "oauth"`,
`data.providerSpecificData.zcodeJwtToken` bắt buộc, không chứa `X-Device-Mid`
(router tự sinh `deviceMid()` trong `zstart.js`).

Kết quả kiểm thử (qa-sprint, 2026-11-07):

- File mẫu với JWT giả 3 phần (`/tmp/qat5/zcode-export.json`): `OK`, exit 0.
- Case lỗi (`provider` sai + JWT 2 phần): `AssertionError`, exit 1 — đúng.
- Không truyền đường dẫn (mặc định `export/zcode-export.json` chưa có):
  trước t3 traceback `FileNotFoundError`; t3 đã cải thiện script in thông báo
  dễ hiểu `LOI: khong thay file export: ...` + exit 1 (đã kiểm chứng lại).

Không phát hiện thêm vấn đề về schema.

## 7. Tiếng Trung trong repo (cập nhật sau t1/t2)

Quét tự động toàn bộ project (CJK Unified Ideographs), loại trừ thư viện ngoài
(`node_modules/`, `.pnpm-store/`, `dist/`, `.agent-teams/`):

- `src/`, `src-tauri/`, `ui/`, `docs/`, `scripts/`, `README.md`, `TODO.md`:
  **0 dong CJK** (cap nhat 2026-11-07 sau t1+t2).
- `ui/aliyun-captcha.js` da duoc dich hoan toan sang tieng Viet (t2);
  `ui/zcode-captcha-pool.js` da xoa (ngoai pham vi 4 tinh nang).
- Cac dong CJK con lai trong `.pnpm-store/` thuoc thu vien ben ngoai, khong phai
  code du an — bo qua khi kiem tra. `.gitignore` da co `node_modules/`,
  `.pnpm-store/`, `dist/`.

## 8. Việc còn lại trên Windows (ngoài phạm vi box Linux)

- Chạy `pnpm tauri dev` / `pnpm tauri build` để xác nhận Rust biên dịch.
- Test claim thật với JWT/WebView captcha (slider/mint).
- Kiểm tra activation body rút gọn (mục 4.2) có bị upstream từ chối không.
- Cân nhắc đồng bộ `appVersion` lên `3.16.3` (qua `ZCODE_APP_VERSION`)
  và thêm `platform` cho balance.

Ghi chú về style: các tài liệu trong `docs/` dùng tiếng Việt không dấu
theo quy ước hiện tại của dự án; file này giữ nguyên quy ước đó.
