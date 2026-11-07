# Source attribution

CLAIM protocol + Rust types ported from Agent2API (fork ngapngap/agent2api, main ec58e127).
- `src-tauri/src/claim.rs` ← `desktop-tauri/src-tauri/server/src/server/core/providers/zcode/claim.rs` (838 lines → minimal port)
- `ui/aliyun-captcha.js` (773 lines) + `ui/zcode-captcha-pool.js` (253 lines) — copied verbatim
- Export schema ← router `src/lib/db/repos/connectionsRepo.js` + `oss/glm.js` (provider=glm, authType=oauth)
- Docs ← `docs/protocol.md` (2 endpoints, headers, error codes 1001–401)
