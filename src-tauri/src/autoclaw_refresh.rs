//! AutoClaw token refresh: tự tạo access token mới từ refresh token.
//!
//! Ported from Agent2API `providers/autoclaw/refresh.rs` (Agent2API phase 2,
//! T-c1). Access token AutoClaw chỉ sống ~1 ngày; refresh token sống dài hơn
//! nhiều. Không có refresh thì app chỉ sống 1 ngày sau mỗi lần OAuth login.
//!
//! Refresh chain (khớp nguồn 1-1):
//! 1. **Single-flight**: cùng 1 tài khoản (id + refresh_token fingerprint)
//!    mà đang có 1 lần refresh chạy thì chờ chung kết quả, không bắn 2 request.
//! 2. `POST {userapi}/userapi/v1/refresh`, headers = `signed_auth_headers`
//!    (X-Auth-Sign = MD5(appId&ts&appKey), xem autoclaw_balance.rs — dùng
//!    chung 1 bản, KHÔNG copy), body `{refresh_token, source_id: "autoclaw", device_id?}`.
//! 3. Nếu `code == 400002` (REFRESH_FALLBACK_CODE, kiểm tra chữ ký thất bại)
//!    -> fallback `POST {userapi}/userapi/v1/agent-refresh`.
//! 4. Kiểm tra `code == 0 && data.access_token` không rỗng; sai -> 401 khi
//!    code 401/410000, còn lại 502.
//! 5. `refresh_token` dùng giá trị mới trong response, thiếu thì giữ cũ.
//! 6. Hạn token tính lại từ JWT `exp` của token mới (không giải được thì
//!    giữ hạn cũ).
//!
//! Gọi refresh ở 2 chỗ (giống Agent2API `adapter.rs`):
//! - **Trước khi gọi**: token sắp hết hạn (trong 5 phút) hoặc đã hết hạn mà có
//!   refresh_token -> refresh trước (ensure_fresh). Token vừa lấy mà bị từ
//!   chối: ép refresh rồi gọi lại 1 lần (force, refresh_access_token).
//! - Chỉ trả credential đã đổi cho caller; caller tựpersist vào
//!   autoclaw-accounts.json (file app này — khác hẳn auth.json của desktop,
//!   ghi vào đó sẽ đè login AutoClaw desktop + xung đột luân phiên token
//!   của desktop client với server, đúng lý do Agent2API `persistRefresh:false`
//!   cho desktop state).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::autoclaw_balance::signed_auth_headers;
use crate::autoclaw_credentials::AutoClawCredentials;
use crate::autoclaw_crypto;

const REFRESH_PATH: &str = "/userapi/v1/refresh";
const AGENT_REFRESH_PATH: &str = "/userapi/v1/agent-refresh";
/// Code kiểm tra chữ ký khi refresh thất bại -> chuyển sang agent-refresh.
/// Khớp Agent2API `REFRESH_FALLBACK_CODE`.
const REFRESH_FALLBACK_CODE: i64 = 400_002;
/// Token còn < 5 phút là refresh trước (PROACTIVE_REFRESH_MARGIN_MS = 300000).
pub(crate) const PROACTIVE_REFRESH_MARGIN_MS: f64 = 300_000.0;
/// Hạn của bản refresh trong bảng single-flight (s là số thực).
const INFLIGHT_TTL_MS: f64 = 60_000.0;

/// Token còn < 5 phút là "sắp hết hạn" (không có hạn -> không coi là sắp hết).
pub fn is_expiring(credentials: &AutoClawCredentials) -> bool {
    match credentials.expires_at {
        Some(expires_at) => {
            expires_at - PROACTIVE_REFRESH_MARGIN_MS <= now_ms()
        }
        None => false,
    }
}

/// Có refresh_token thì mới refresh được (tài khoản đọc từ openclaw.json
/// không có refresh token -> không bao giờ refresh được).
pub fn can_refresh(credentials: &AutoClawCredentials) -> bool {
    !credentials.refresh_token.trim().is_empty()
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

fn fingerprint(text: &str) -> String {
    // SHA-256 rút gọn: chỉ dùng làm key, không cần crypto.
    // md5 0.7 không expose sha256; dùng md5 hex là đủ phân biệt (không mang
    // giá trị bảo mật — refresh_token không bao giờ vào key/log).
    let digest = md5::compute(text.as_bytes());
    format!("{:x}", digest)
}

/// Bảng single-flight: key -> timestamp (ms) của lần refresh đang chạy.
/// Quá hạn 60s thì coi là chết (request 30s timeout mà không về thì bỏ).
fn inflight() -> &'static Mutex<HashMap<String, f64>> {
    static TABLE: OnceLock<Mutex<HashMap<String, f64>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn refresh_key(credentials: &AutoClawCredentials) -> String {
    format!(
        "{}:{}:{}",
        credentials.id,
        credentials.region.label(),
        fingerprint(credentials.refresh_token.trim())
    )
}

enum Claim {
    /// Mình là leader (đã ghi key vào bảng), caller tự bắn request.
    Leader,
    /// Đã có leader khác đang refresh, caller sleep rồi đọc lại cache.
    Waiter,
}

fn claim(key: &str) -> Claim {
    let mut guard = inflight().lock().unwrap();
    let now = now_ms();
    if let Some(&started) = guard.get(key) {
        if now - started < INFLIGHT_TTL_MS {
            return Claim::Waiter;
        }
    }
    guard.insert(key.to_string(), now);
    Claim::Leader
}

fn release(key: &str) {
    if let Ok(mut guard) = inflight().lock() {
        guard.remove(key);
    }
}

/// Cache kết quả refresh trong tiến trình (key = refresh_key của credential
/// cũ): waiter lần 2 không cần refresh lại, lấy luôn token mới.
/// Client desktop login lại thì file đổi -> lần gọi sau đọc credential mới
/// -> key khác -> cache cũ tự hết dùng (không ảnh hưởng).
fn cache() -> &'static Mutex<HashMap<String, (AutoClawCredentials, f64)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (AutoClawCredentials, f64)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached(key: &str) -> Option<AutoClawCredentials> {
    if let Ok(guard) = cache().lock() {
        if let Some((creds, at)) = guard.get(key) {
            if now_ms() - *at < 10 * 60_000.0 {
                return Some(creds.clone());
            }
        }
    }
    None
}

fn store_cached(key: &str, credentials: &AutoClawCredentials) {
    if let Ok(mut guard) = cache().lock() {
        guard.insert(key.to_string(), (credentials.clone(), now_ms()));
    }
}

/// Làm mới credential: force=false chỉ refresh khi sắp hết hạn/đã hết hạn
/// (ensure_fresh); force=true luôn refresh (dùng khi request bị 401 mà token
/// thời gian còn mới — chỉ xem hạn sẽ lấy lại đúng token hỏng đó).
/// Single-flight: chờ leader xong rồi lấy kết quả từ cache, không gọi lại.
pub async fn refresh(
    credentials: &AutoClawCredentials,
    force: bool,
) -> Result<AutoClawCredentials, String> {
    if !force && !is_expiring(credentials) {
        return Ok(credentials.clone());
    }
    if !can_refresh(credentials) {
        return Err(
            "Tài khoản AutoClaw chưa có refresh token, không thể tự làm mới — đăng nhập lại AutoClaw"
                .to_string(),
        );
    }
    let key = refresh_key(credentials);
    // Từng có kết quả rất gần đây của đúng key này (leader vừa xong):
    // lấy luôn, khỏi bắn request thứ 2.
    if let Some(creds) = cached(&key) {
        return Ok(creds);
    }
    match claim(&key) {
        Claim::Waiter => {
            // Chờ leader khoảng 32s rồi kiểm tra cache (request timeout 30s).
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(32);
            while std::time::Instant::now() < deadline {
                if let Some(creds) = cached(&key) {
                    return Ok(creds);
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            // Leader không về được: thử bắn request bản thân (key đã tự hết
            // hạn sau 60s, nhưng vào đây nên bỏ qua giữ chỗ cũ).
            release(&key);
            let _ = claim(&key);
        }
        Claim::Leader => {}
    }
    let result = call_refresh_api_with_fallback(credentials).await;
    release(&key);
    match result {
        Ok(next) => {
            store_cached(&key, &next);
            Ok(next)
        }
        Err(error) => Err(error),
    }
}

/// Chỉ refresh khi token sắp hết hạn (gọi trước request).
pub async fn ensure_fresh(
    credentials: &AutoClawCredentials,
) -> Result<AutoClawCredentials, String> {
    if !is_expiring(credentials) || !can_refresh(credentials) {
        return Ok(credentials.clone());
    }
    refresh(credentials, false).await
}

fn biz_code(payload: &Value) -> Option<i64> {
    match payload.get("code") {
        Some(Value::Number(number)) => number.as_i64().or_else(|| number.as_f64().map(|v| v as i64)),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn js_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        other => other.to_string(),
    }
}

/// Gọi 1 lần refresh endpoint: POST {userapi}{path} với signed headers,
/// body {refresh_token, source_id: "autoclaw", device_id?}.
/// Không đi qua proxy tài khoản: userapi domain là domain xác thực, không
/// phải LLM domain (khớp Agent2API).
async fn call_refresh_api(
    credentials: &AutoClawCredentials,
    path: &str,
) -> Result<Value, String> {
    let base = credentials.region.userapi_base_url();
    let url = format!("{}{}", base, path);
    let headers = signed_auth_headers(&credentials.token);
    let mut body = json!({
        "refresh_token": credentials.refresh_token,
        "source_id": "autoclaw",
    });
    if !credentials.device_id.trim().is_empty() {
        if let Some(map) = body.as_object_mut() {
            map.insert(
                "device_id".to_string(),
                Value::String(credentials.device_id.clone()),
            );
        }
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("AutoClaw refresh không tạo được client: {}", e))?;
    let mut req = client.post(&url).json(&body);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!("AutoClaw refresh timeout ({})", path)
            } else {
                format!("AutoClaw refresh thất bại ({}): {}", path, e)
            }
        })?;
    let status = resp.status().as_u16();
    let payload: Value = resp.json().await.unwrap_or(Value::Null);
    if status == 401 {
        return Err("Phien dang nhap het han va refresh token cung khong con dung duoc — hay dang nhap lai AutoClaw".to_string());
    }
    if !(200..300).contains(&status) {
        return Err(format!("AutoClaw refresh tra ve HTTP {} ({})", status, path));
    }
    Ok(payload)
}

/// Refresh entry: thử refresh, gặp code 400002 thì fallback agent-refresh
/// rồi kiểm tra code 0 + access_token, tính lại hạn từ JWT mới.
async fn call_refresh_api_with_fallback(
    credentials: &AutoClawCredentials,
) -> Result<AutoClawCredentials, String> {
    let mut payload = call_refresh_api(credentials, REFRESH_PATH).await?;
    if biz_code(&payload) == Some(REFRESH_FALLBACK_CODE) {
        payload = call_refresh_api(credentials, AGENT_REFRESH_PATH).await?;
    }
    let code = biz_code(&payload);
    let data = payload.get("data").cloned().unwrap_or(Value::Null);
    let access_token = data
        .get("access_token")
        .map(js_text)
        .map(|text| autoclaw_crypto::strip_bearer(&text))
        .unwrap_or_default();
    if code != Some(0) || access_token.trim().is_empty() {
        let unauthorized = code == Some(401) || code == Some(410_000);
        let msg = payload
            .get("msg")
            .or_else(|| payload.get("message"))
            .map(js_text)
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "server khong tra ve access token moi".to_string());
        if unauthorized {
            return Err(format!(
                "Phien dang nhap het han va refresh token cung khong con dung duoc (code={}): {} — hay dang nhap lai AutoClaw",
                payload.get("code").map(js_text).unwrap_or_else(|| "null".to_string()),
                msg
            ));
        }
        return Err(format!(
            "AutoClaw refresh token that bai (code={}): {}",
            payload.get("code").map(js_text).unwrap_or_else(|| "null".to_string()),
            msg
        ));
    }
    let next_refresh = data
        .get("refresh_token")
        .map(js_text)
        .map(|text| autoclaw_crypto::strip_bearer(&text))
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| credentials.refresh_token.clone());
    let claims = autoclaw_crypto::decode_jwt_claims(&access_token);
    let mut next = credentials.clone();
    next.token = access_token;
    next.refresh_token = next_refresh;
    // Hạn tính lại từ JWT mới; giải không được thì giữ hạn cũ (khớp nguồn).
    if let Some(exp) = claims
        .as_ref()
        .and_then(|c| c.get("exp"))
        .and_then(|v| match v {
            Value::Number(n) => n.as_f64().filter(|x| *x > 0.0),
            Value::String(t) => t.trim().parse::<f64>().ok().filter(|x| *x > 0.0),
            _ => None,
        })
    {
        next.expires_at = Some(exp * 1000.0);
    }
    if next.user_id.trim().is_empty() {
        if let Some(uid) = claims
            .as_ref()
            .and_then(|c| c.get("user_id"))
            .map(js_text)
        {
            next.user_id = uid;
        }
    }
    if next.device_id.trim().is_empty() {
        if let Some(did) = claims
            .as_ref()
            .and_then(|c| c.get("device_id"))
            .map(js_text)
        {
            next.device_id = did;
        }
    }
    Ok(next)
}
