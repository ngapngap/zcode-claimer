//! ZCode dang nhap: luong CLI poll (giong Agent2API
//! `providers/zcode/oauth.rs`):
//!
//! 1. Tao poll token (32 byte ngau nhien hex) dung chung Bearer cho init va poll
//! 2. POST https://zcode.z.ai/api/v1/oauth/cli/init
//!    body {"provider": "zai"} -> {flow_id, authorize_url, expires_at, poll_interval_sec}
//! 3. authorize_url duoc them tham so trang trung gian (ban quooc te = redirect_uri)
//!    roi giao cho frontend mo trinh duyet mac dinh
//! 4. GET /api/v1/oauth/cli/poll/{flow_id} moi poll_interval_sec mot lan,
//!    den khi status=ready -> {token, user.user_id, data.zai.access_token}
//!
//! Ban quooc te (Intl): provider="zai", param=redirect_uri, doc data.zai.
//! Giong Agent2API Region::Intl. Base URL zcode.z.ai giu nguyen nhu Agent2API.
//!
//! Da bo: LoginService task table (frontend poll truc tiep qua Tauri State),
//! coding_key::resolve (chi can JWT/deviceMid/userId). Username/email thu cong.
//!
//! Ngu nghia loi (theo Agent2API): 5xx/408/429/mat mang/200 sai body -> pending;
//! 4xx con lai, envelope code!=0, failed -> fatal, ket thuc ngay va bao loi.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claim;

const DEFAULT_POLL_INTERVAL_SEC: i64 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginStart {
    pub flow_id: String,
    pub auth_url: String,
    pub expires_at: Option<i64>,
    pub poll_interval_sec: i64,
}

/// Ket qua mot lan poll (None = chua xac thuc xong; Err = fatal)
#[derive(Clone, Debug)]
pub enum PollResult {
    Pending,
    Ready(ZcodeLoginCredentials),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZcodeLoginCredentials {
    pub jwt: String,
    pub access_token: String,
    pub user_id: String,
    pub device_mid: String,
}

#[derive(Clone, Debug)]
pub struct CliLogin {
    pub flow_id: String,
    pub poll_token: String,
    pub poll_interval_sec: i64,
    pub expires_at: Option<i64>,
    pub device_mid: String,
}

fn region_provider() -> &'static str {
    "zai"
}

fn app_version() -> String {
    claim::app_version()
}

fn random_hex_32() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        let fallback = format!(
            "{:x}{:x}{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|v| v.as_nanos())
                .unwrap_or(0),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|v| v.subsec_nanos())
                .unwrap_or(0)
        );
        return format!("{fallback:0>64}").chars().take(64).collect();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn envelope_error(payload: &Value) -> Option<String> {
    let code = payload.get("code").and_then(Value::as_i64)?;
    if code == 0 {
        return None;
    }
    let message = payload
        .get("msg")
        .or_else(|| payload.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("Upstream tu choi");
    Some(format!("ZCode dang nhap that bai ({code}): {message}"))
}

/// Giong Agent2API `apply_interstitial`: semantics "set" (thay doi cung ten),
/// encode toan bo (inner `zcode%3A%2F%2F` encode lai thanh `%253A%252F%252F`).
/// Ban quooc te dung param `redirect_uri`, ban Trung Quoc dung `redirect`.
fn apply_interstitial(authorize_url: &str) -> String {
    let param = "redirect_uri";
    let target = claim::urlencode(&format!(
        "https://zcode.z.ai/app/oauth/login?redirect=zcode%3A%2F%2Foauth%2Fcallback&app_version={}",
        app_version()
    ));
    let prefix = format!("{param}=");
    let replacement = format!("{prefix}{target}");
    let (head, fragment) = match authorize_url.split_once('#') {
        Some((head, fragment)) => (head, Some(fragment)),
        None => (authorize_url, None),
    };
    let (path, query) = match head.split_once('?') {
        Some((path, query)) => (path, query),
        None => (head, ""),
    };
    let mut replaced = false;
    let mut segments: Vec<String> = Vec::new();
    for segment in query.split('&').filter(|v| !v.is_empty()) {
        if segment.starts_with(&prefix) {
            if !replaced {
                segments.push(replacement.clone());
                replaced = true;
            }
            continue;
        }
        segments.push(segment.to_string());
    }
    if !replaced {
        segments.push(replacement);
    }
    let mut out = String::from(path);
    out.push('?');
    out.push_str(&segments.join("&"));
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

fn required_str(data: &Value, key: &str) -> Result<String, String> {
    data.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("ZCode dang nhap: thieu {key} trong phan hoi"))
}

/// Khoi tao dang nhap: init 1 lan, lay flow_id + auth_url roi giao ve frontend mo browser.
/// `device_mid` do caller (main.rs) cung cap tu persistence de on dinh theo account.
pub async fn start(device_mid: &str) -> Result<(LoginStart, CliLogin), String> {
    let device_mid = device_mid.trim().to_string();
    if device_mid.is_empty() {
        return Err("ZCode dang nhap: thieu device_mid (can on dinh theo account)".to_string());
    }
    let poll_token = random_hex_32();
    let url = "https://zcode.z.ai/api/v1/oauth/cli/init";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(15_000))
        .build()
        .map_err(|e| e.to_string())?;
    let body = serde_json::json!({ "provider": region_provider() });
    let response = client
        .post(url)
        .header("authorization", format!("Bearer {poll_token}"))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|_| "ZCode dang nhap khoi tao: timeout hoac loi mang".to_string())?;
    let payload: Value = response
        .json()
        .await
        .map_err(|_| "ZCode dang nhap: phan hoi khong phai JSON".to_string())?;
    if let Some(message) = envelope_error(&payload) {
        return Err(message);
    }
    let data = payload
        .get("data")
        .ok_or_else(|| "ZCode dang nhap: thieu data".to_string())?;
    let flow_id = required_str(data, "flow_id")?;
    let authorize_url = required_str(data, "authorize_url")?;
    let expires_at = data.get("expires_at").and_then(Value::as_i64);
    let poll_interval_sec = data
        .get("poll_interval_sec")
        .and_then(Value::as_i64)
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_POLL_INTERVAL_SEC);

    let login = CliLogin {
        flow_id: flow_id.clone(),
        poll_token,
        poll_interval_sec,
        expires_at,
        device_mid,
    };
    let auth_url = apply_interstitial(&authorize_url);
    Ok((
        LoginStart {
            flow_id,
            auth_url,
            expires_at,
            poll_interval_sec,
        },
        login,
    ))
}

/// Poll 1 lan (mang/5xx/408/429/200 sai body -> Pending; con lai fatal Err)
pub async fn poll(login: &CliLogin) -> Result<PollResult, String> {
    let url = format!("https://zcode.z.ai/api/v1/oauth/cli/poll/{}", login.flow_id);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(15_000))
        .build()
        .map_err(|e| e.to_string())?;
    let response = match client
        .get(&url)
        .header("authorization", format!("Bearer {}", login.poll_token))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return Ok(PollResult::Pending),
    };
    let status = response.status().as_u16();
    if status >= 500 || status == 408 || status == 429 {
        return Ok(PollResult::Pending);
    }
    let payload: Value = match response.json().await {
        Ok(v) => v,
        Err(_) => return Ok(PollResult::Pending),
    };
    if let Some(message) = envelope_error(&payload) {
        return Err(message);
    }
    if status >= 400 {
        return Err(format!("ZCode dang nhap bi tu choi (HTTP {status})"));
    }
    let data = match payload.get("data") {
        Some(d) => d,
        None => return Ok(PollResult::Pending),
    };
    let s = data.get("status").and_then(Value::as_str).unwrap_or("");
    match s {
        "ready" => {
            let key = region_provider();
            let access_token = data
                .get(key)
                .and_then(|v| v.get("access_token"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("ZCode dang nhap: thieu {key}.access_token"))?;
            let jwt = data
                .get("token")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .to_string();
            if jwt.is_empty() {
                return Err("ZCode dang nhap: token rong khi ready".to_string());
            }
            let user_id = data
                .get("user")
                .and_then(|v| v.get("user_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .to_string();
            Ok(PollResult::Ready(ZcodeLoginCredentials {
                jwt,
                access_token,
                user_id,
                device_mid: login.device_mid.clone(),
            }))
        }
        "failed" => Err("ZCode quyen bi tu choi hoac het hieu luc, hay dang nhap lai".to_string()),
        _ => Ok(PollResult::Pending),
    }
}
