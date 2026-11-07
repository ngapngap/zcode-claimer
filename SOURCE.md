# Source attribution

CLAIM protocol + Rust types (port theo protocol ZCode).
- `src-tauri/src/claim.rs` (838 dòng bản gốc → minimal port)
- `ui/aliyun-captcha.js` (773 lines) + `ui/zcode-captcha-pool.js` (253 lines) — copied verbatim
- Export schema ← router `src/lib/db/repos/connectionsRepo.js` + `oss/glm.js` (provider=glm, authType=oauth)
- Docs ← `docs/protocol.md` (2 endpoints, headers, error codes 1001–401)
