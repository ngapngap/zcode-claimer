//! ZCode 登录：CLI 轮询流程（自 Agent2API
//! `desktop-tauri/src-tauri/server/src/server/core/providers/zcode/oauth.rs`
//! + `src/server/core/login/zcode.rs` 移植，核心逻辑逐字守住）：
//!
//! 1. 生成 poll token（32 随机字节 hex）—— init 与 poll 共用 Bearer
//! 2. POST https://zcode.z.ai/api/v1/oauth/cli/init
//!    body {"provider": "bigmodel"|"zai"} → {flow_id, authorize_url, expires_at, poll_interval_sec}
//! 3. authorize_url 补中转页参数（两地参数名不同：Intl=redirect_uri, Cn=redirect）
//!    后交给前端打开系统浏览器 —— 浏览器不会回到本机，授权在服务端记完
//! 4. GET /api/v1/oauth/cli/poll/{flow_id} 每 poll_interval_sec 一次，
//!    直到 status=ready → {token, user.user_id, data[zai|bigmodel].access_token}
//!
//! 移植时删掉的：LoginService 任务表（前端直接用 Tauri State + 前端轮询
//! + 5 分钟本地超时兜住）、coding_key::resolve（那是推理凭证换取，claimer
//! 只需要 JWT/deviceMid/userId，没有推理转发；username/email 官方登录
//! 本来就不返回，沿用本 claimer 现有的手填字段）。
//!
//! 错误语义（逐条照 Agent2API）：5xx/408/429/断网/200 畸形体 → pending；
//! 4xx 其余、信封 code!=0、failed → 致命，立刻结束并报错。

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

/// 一次轮询的结果（None = 还没授权完；Err = 致命失败）
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

pub struct CliLogin {
    pub flow_id: String,
    pub poll_token: String,
    pub poll_interval_sec: i64,
    pub expires_at: Option<i64>,
    pub device_mid: String,
}

/// Helper public de main.rs reconstruct tu State (single-flight, khong expose qua invoke).
pub struct CliLoginSendable {
    pub flow_id: String,
    pub poll_token: String,
    pub device_mid: String,
}

pub async fn poll_sendable(login: &CliLoginSendable) -> Result<PollResult, String> {
    let tmp = CliLogin {
        flow_id: login.flow_id.clone(),
        poll_token: login.poll_token.clone(),
        poll_interval_sec: 3,
        expires_at: None,
        device_mid: login.device_mid.clone(),
    };
    poll(&tmp).await
}

fn region_provider() -> &'static str {
    "bigmodel"
}

fn app_version() -> String {
    claim::app_version()
}

fn random_hex_32() -> String {
    let mut bytes = [0u8; 32];
    // 取不到随机数时回落到时间戳串：poll token 只是本次流程关联标识，
    // 上游不拿它做鉴权，只用来把 init 与 poll 归到同一 flow。
    // CSPRNG: dùng getrandom::fill (API cua getrandom 0.3).
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

/// 与 Agent2API 的 `apply_interstitial` 逐字一致：语义是 "set"（同名替换）、
/// 整体编码（内层 `zcode%3A%2F%2F` 再编一次得 `%253A%252F%252F`）。
fn apply_interstitial(authorize_url: &str) -> String {
    let param = "redirect"; // Cn. Intl tai day bi loai; neu mo lai thi them redirect_uri.
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

/// 发起登录：init 一次，拿到 flow_id + auth_url 后立刻交回前端打开浏览器。
pub async fn start() -> Result<(LoginStart, CliLogin), String> {
    let poll_token = random_hex_32();
    let url = format!("https://zcode.z.ai/api/v1/oauth/cli/init");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(15_000))
        .build()
        .map_err(|e| e.to_string())?;
    let body = serde_json::json!({ "provider": region_provider() });
    let response = client
        .post(&url)
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

    let token_state = std::sync::Arc::new(tokio::sync::Mutex::new(poll_token.clone()));
    // device_mid 与凭证随落盘（跨请求稳定）：login 时生成一次，poll 时带走。
    let device_mid = uuid::Uuid::new_v4().to_string();

    let login = CliLogin {
        flow_id: flow_id.clone(),
        poll_token,
        poll_interval_sec,
        expires_at,
        device_mid: device_mid.clone(),
    };
    let _ = token_state;
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

/// 轮询一次（网络/5xx/408/429/200 畸形体 → Pending；其余致命 Err）
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
