//! AutoClaw balance (POINTS - diem). Ported from the AutoClaw desktop protocol.
//! All requests via userapi domain with signed headers + Bearer.
//! - GET {userapi}/agent-assetmgr/api/v2/wallets?biz_app_id=autoclaw (main)
//! - fallback GET /agent-assetmgr/api/v1/wallet-instances?wallet_type=all&wallet_scope=all
//! - GET /agent-assetmgr/api/v1/points/expiring?biz_app_id=autoclaw (non-blocking)
//! - POST {userapi}/agentpay/v1/assistant/subscribe-info (Cn) or /oversea-subscribe-info (Intl)
//! Timeout 20s. Unit = "Điểm" (diem).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::autoclaw_credentials::AutoClawCredentials;
use crate::autoclaw_crypto;

pub const REQUEST_TIMEOUT_SECS: u64 = 20;
const POINTS_WALLET_PATH: &str = "/agent-assetmgr/api/v2/wallets?biz_app_id=autoclaw";
const POINTS_WALLET_LEGACY_PATH: &str =
    "/agent-assetmgr/api/v1/wallet-instances?wallet_type=all&wallet_scope=all";
const POINTS_EXPIRING_PATH: &str = "/agent-assetmgr/api/v1/points/expiring?biz_app_id=autoclaw";
const AUTH_EXPIRED_CODES: &[i64] = &[410_000, 400_000];

const AUTH_APP_ID: &str = "100003";
const AUTH_APP_KEY: &str = "38d2391985e2369a5fb8227d8e6cd5e5";
const CLIENT_VERSION: &str = "1.18.5";

const WALLET_SCOPE_LABELS: &[(&str, &str)] = &[
    ("daily", "Điểm tặng"),
    ("monthly", "Điểm gói"),
    ("long_term", "Điểm dùng chung"),
    ("annual", "Điểm dùng chung"),
    ("campaign", "Điểm sự kiện"),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutoClawBucket {
    pub bucket_id: String,
    pub show_name: String,
    pub remaining_points: f64,
    pub balance_view: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutoClawBalanceResult {
    pub available: Value,
    pub unit: String,
    pub wallets: Vec<AutoClawBucket>,
    pub subscription: Value,
    pub expiring: Value,
    pub raw: Value,
    /// User id + email of the credential used (from JWT claims / auth.json),
    /// so the frontend can show the AutoClaw login as its own account row.
    pub user_id: String,
    pub email: String,
}

/// Ham ky dung chung cho moi request userapi AutoClaw (balance / refresh /
/// oauth): appId/appKey cua client + timestamp giay + MD5. Giu 1 ban duy nhat
/// o day (dung chung 1 ban) — oauth.rs dung lai, khong copy rieng.
pub(crate) fn signed_auth_headers(token: &str) -> Vec<(String, String)> {
    let timestamp = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)) as i64;
    let ts = timestamp.max(0).to_string();
    let digest = md5::compute(format!("{}&{}&{}", AUTH_APP_ID, ts, AUTH_APP_KEY).as_bytes());
    let sign = format!("{:x}", digest);
    let trace = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{:x}{:x}", nanos, timestamp)
    };
    let mut headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Accept".to_string(), "*/*".to_string()),
        ("X-Version".to_string(), CLIENT_VERSION.to_string()),
        ("X-Product".to_string(), "autoclaw".to_string()),
        ("X-Client-Type".to_string(), "pc".to_string()),
        ("X-Harness-Type".to_string(), "zcode".to_string()),
        ("X-Tm".to_string(), if cfg!(windows) { "win" } else { "linux" }.to_string()),
        ("X-Lang".to_string(), "zh-CN".to_string()),
        ("X-Channel".to_string(), "official".to_string()),
        ("X-Auth-Appid".to_string(), AUTH_APP_ID.to_string()),
        ("X-Auth-TimeStamp".to_string(), ts),
        ("X-Auth-Sign".to_string(), sign),
        ("X-Trace-Id".to_string(), trace),
    ];
    if !token.is_empty() {
        // Boc 1 lan "Bearer " truoc khi ghep (tranh "Bearer Bearer ..."):
        // token tu login co the da mang san prefix, ghep them se thanh
        // "Bearer Bearer ..." -> upstream cat ra token sai -> 401 co dinh.
        let bare = autoclaw_crypto::strip_bearer(token);
        if !bare.is_empty() {
            headers.push(("authorization".to_string(), format!("Bearer {bare}")));
        }
    }
    headers
}

async fn userapi_get(
    credentials: &AutoClawCredentials,
    path: &str,
    what: &str,
) -> Result<Value, String> {
    let url = format!("{}{}", credentials.region.userapi_base_url(), path);
    let headers = signed_auth_headers(&credentials.token);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("{} that bai: {}", what, e))?;
    let mut req = client.get(&url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|e| {
        if e.is_timeout() { format!("{} timeout", what) } else { format!("{} that bai: {}", what, e) }
    })?;
    let status = resp.status().as_u16();
    let payload: Value = resp.json().await.unwrap_or(Value::Null);
    if status == 401 {
        return Err(format!("Dang nhap het han, khong the {}", what));
    }
    if !(200..300).contains(&status) {
        return Err(format!("{} tra ve HTTP {}", what, status));
    }
    Ok(payload)
}

async fn userapi_post(
    credentials: &AutoClawCredentials,
    path: &str,
    body: &Value,
    what: &str,
) -> Result<Value, String> {
    let url = format!("{}{}", credentials.region.userapi_base_url(), path);
    let headers = signed_auth_headers(&credentials.token);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("{} that bai: {}", what, e))?;
    let mut req = client.post(&url).json(body);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|e| {
        if e.is_timeout() { format!("{} timeout", what) } else { format!("{} that bai: {}", what, e) }
    })?;
    let status = resp.status().as_u16();
    let payload: Value = resp.json().await.unwrap_or(Value::Null);
    if status == 401 {
        return Err(format!("Dang nhap het han, khong the {}", what));
    }
    if !(200..300).contains(&status) {
        return Err(format!("{} tra ve HTTP {}", what, status));
    }
    Ok(payload)
}

fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        _ => false,
    }
}

pub(crate) fn js_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().map(|i| i != 0.0).unwrap_or(false),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(_)) => true,
    }
}

pub(crate) fn number_or_null(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64().filter(|i| i.is_finite()),
        Value::String(text) => text.trim().parse::<f64>().ok().filter(|i| i.is_finite()),
        _ => None,
    }
}

fn json_number_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 9.007_199_254_740_992e15 {
        format!("{}", value as i64)
    } else {
        format!("{}", value)
    }
}

fn subscribe_info_path(credentials: &AutoClawCredentials) -> &'static str {
    match credentials.region {
        crate::autoclaw_credentials::Region::Cn => "/agentpay/v1/assistant/subscribe-info",
        crate::autoclaw_credentials::Region::Intl => "/agentpay/v1/assistant/oversea-subscribe-info",
    }
}

async fn query_subscription(credentials: &AutoClawCredentials) -> Result<Value, String> {
    let body = if credentials.device_id.is_empty() {
        json!({ "source_id": "autoclaw" })
    } else {
        json!({ "source_id": "autoclaw", "device_id": credentials.device_id })
    };
    let payload = userapi_post(
        credentials,
        subscribe_info_path(credentials),
        &body,
        "Truy van goi su dung",
    )
    .await?;
    if let Some(code) = payload.get("code").and_then(Value::as_i64) {
        if AUTH_EXPIRED_CODES.contains(&code) {
            return Err("Dang nhap het han, khong the truy van goi su dung".to_string());
        }
        if code != 0 {
            let msg = payload.get("msg").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string)
                .unwrap_or_else(|| format!("Truy van goi su dung tra ve bat thuong code={}", code));
            return Err(msg);
        }
    }
    let data = payload.get("data").cloned().unwrap_or(Value::Null);
    if data.is_null() {
        return Ok(Value::Null);
    }
    Ok(normalize_subscription(&data))
}

fn normalize_subscription(data: &Value) -> Value {
    let nested = ["subscribeInfo", "subscribe", "data", "memberInfo", "vipInfo"]
        .iter()
        .filter_map(|key| data.get(*key))
        .find(|v| v.is_object());
    let mut roots: Vec<&Value> = vec![data];
    if let Some(n) = nested {
        roots.push(n);
    }
    fn find_deep(roots: &[&Value], keys: &[&str]) -> Option<Value> {
        let mut pending: Vec<&Value> = roots.to_vec();
        let mut visited = 0usize;
        while let Some(current) = pending.pop() {
            visited += 1;
            if visited > 200 {
                break;
            }
            let Some(obj) = current.as_object() else { continue };
            for (k, v) in obj {
                if keys.contains(&k.as_str()) && !is_blank(v) {
                    return Some(v.clone());
                }
                if v.is_object() {
                    pending.push(v);
                }
            }
        }
        None
    }
    let text_of = |v: Option<Value>| -> String {
        match v {
            Some(Value::String(t)) if !t.trim().is_empty() => t.trim().to_string(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        }
    };
    let quota_of = |keys: &[&str]| -> Value {
        find_deep(&roots, keys)
            .and_then(|v| match v {
                Value::Number(n) => n.as_f64(),
                Value::String(t) => t.trim().parse::<f64>().ok(),
                _ => None,
            })
            .filter(|v| v.is_finite())
            .map(|v| json!(v))
            .unwrap_or(Value::Null)
    };
    json!({
        "planName": text_of(find_deep(&roots, &["productName", "planName", "memberName", "subscribeName", "vipName"])),
        "status": text_of(find_deep(&roots, &["status", "subscribeStatus", "memberStatus"])),
        "expireAt": find_deep(&roots, &["expireAt", "expireTime", "expiredAt", "endTime", "validEndTime"]).unwrap_or(Value::Null),
        "remainQuota": quota_of(&["remainQuota", "remainingQuota", "remainCredits", "availableCredits", "leftQuota"]),
        "totalQuota": quota_of(&["totalQuota", "credits", "totalCredits"]),
    })
}

fn normalize_legacy_wallets(instances: &[Value]) -> Vec<Value> {
    let mut merged: Vec<(String, f64)> = Vec::new();
    for instance in instances {
        let status = instance.get("status").and_then(Value::as_str).unwrap_or("").to_lowercase();
        if status != "active" || !js_truthy(instance.get("display")) {
            continue;
        }
        let scope = instance
            .get("wallet_scope")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase()
            .replace("wallet_scope_", "");
        let scope = if scope.is_empty() { "other".to_string() } else { scope };
        let balance = number_or_null(instance.get("balance")).unwrap_or(0.0);
        match merged.iter_mut().find(|(k, _)| k == &scope) {
            Some((_, total)) => *total += balance,
            None => merged.push((scope, balance)),
        }
    }
    merged
        .into_iter()
        .map(|(kind, balance)| {
            let display_name = WALLET_SCOPE_LABELS
                .iter()
                .find(|(s, _)| *s == kind)
                .map(|(_, label)| (*label).to_string())
                .unwrap_or_else(|| "Khác".to_string());
            json!({
                "type": kind,
                "displayName": display_name,
                "balance": balance,
                "balanceView": json_number_text(balance),
            })
        })
        .collect()
}

async fn query_points(credentials: &AutoClawCredentials) -> Result<Value, String> {
    let wallet_payload = userapi_get(credentials, POINTS_WALLET_PATH, "Truy van diem").await?;
    if let Some(code) = wallet_payload.get("code").and_then(Value::as_i64) {
        if AUTH_EXPIRED_CODES.contains(&code) {
            return Err("Dang nhap het han, khong the truy van diem".to_string());
        }
    }
    let data = wallet_payload.get("data").cloned().unwrap_or(Value::Null);
    let code_ok = wallet_payload.get("code").and_then(Value::as_i64) == Some(0);
    let v2_error = match wallet_payload.get("code").and_then(Value::as_i64) {
        Some(c) if c != 0 => Some(format!("v2 code={} {}", c,
            wallet_payload.get("msg").and_then(Value::as_str).unwrap_or(""))),
        _ => None,
    };
    let (total_balance, wallets, raw) = if code_ok
        && data.get("wallets").and_then(Value::as_array).is_some()
    {
        let arr = data.get("wallets").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut mapped: Vec<(i64, Value)> = arr
            .iter()
            .filter(|w| js_truthy(w.get("display")))
            .map(|w| {
                let priority = w.get("priority").and_then(Value::as_i64).unwrap_or(i64::MAX);
                let entry = json!({
                    "type": w.get("public_wallet_type").cloned().unwrap_or(Value::Null),
                    "displayName": w.get("display_name").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string)
                        .or_else(|| w.get("public_wallet_type").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string))
                        .unwrap_or_else(|| "Điểm".to_string()),
                    "balance": number_or_null(w.get("balance")),
                    "balanceView": w.get("balance_view").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string)
                        .or_else(|| w.get("balance").and_then(Value::as_f64).map(json_number_text))
                        .unwrap_or_default(),
                });
                (priority, entry)
            })
            .collect();
        mapped.sort_by_key(|(p, _)| *p);
        (
            data.get("total_balance").cloned().unwrap_or(Value::Null),
            mapped.into_iter().map(|(_, e)| e).collect::<Vec<_>>(),
            data.clone(),
        )
    } else {
        let legacy = userapi_get(credentials, POINTS_WALLET_LEGACY_PATH, "Truy van diem").await?;
        if let Some(code) = legacy.get("code").and_then(Value::as_i64) {
            if AUTH_EXPIRED_CODES.contains(&code) {
                return Err("Dang nhap het han, khong the truy van diem".to_string());
            }
        }
        let legacy_data = legacy.get("data").cloned().unwrap_or(Value::Null);
        if legacy.get("code").and_then(Value::as_i64) != Some(0) || legacy_data.is_null() {
            let mut msg = legacy.get("msg").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string)
                .unwrap_or_else(|| format!(
                    "Truy van diem tra ve bat thuong code={}",
                    legacy.get("code").and_then(Value::as_i64)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "null".to_string())
                ));
            if let Some(v2_err) = v2_error {
                msg = format!("{} (v2: {})", msg, v2_err);
            }
            return Err(msg);
        }
        let instances = legacy_data.get("wallet_instances").and_then(Value::as_array).cloned().unwrap_or_default();
        (
            legacy_data.get("total_balance").cloned().unwrap_or(Value::Null),
            normalize_legacy_wallets(&instances),
            legacy_data,
        )
    };

    let expiring = match userapi_get(credentials, POINTS_EXPIRING_PATH, "Truy van diem").await {
        Ok(payload) => {
            let d = payload.get("data").cloned().unwrap_or(Value::Null);
            if payload.get("code").and_then(Value::as_i64) == Some(0) && !d.is_null() {
                json!({
                    "points": d.get("expiring_points").cloned().unwrap_or(Value::Null),
                    "text": d.get("expiring_points_text").cloned().unwrap_or(Value::Null),
                    "totalPoints": d.get("total_points").cloned().unwrap_or(Value::Null),
                    "expireTimeValue": d.get("expire_time_value").cloned().unwrap_or(Value::Null),
                    "expireTimeType": d.get("expire_time_type").cloned().unwrap_or(Value::Null),
                })
            } else {
                Value::Null
            }
        }
        Err(_) => Value::Null,
    };

    Ok(json!({
        "totalBalance": total_balance,
        "wallets": wallets,
        "expiring": expiring,
        "raw": raw,
    }))
}

pub async fn query_balance(credentials: &AutoClawCredentials) -> Result<AutoClawBalanceResult, String> {
    if credentials.token.trim().is_empty() {
        return Err("Khong co token, khong the truy van diem".to_string());
    }
    let points = query_points(credentials).await?;
    let (subscription, subscription_error) = match query_subscription(credentials).await {
        Ok(v) => (v, Value::Null),
        Err(e) => (Value::Null, Value::String(e)),
    };
    let wallets_raw = points.get("wallets").cloned().unwrap_or_else(|| Value::Array(Vec::new()));
    let expiring = points.get("expiring").cloned().unwrap_or(Value::Null);
    let mut wallets: Vec<AutoClawBucket> = Vec::new();
    let mut wallets_sum = 0.0;
    if let Some(arr) = wallets_raw.as_array() {
        for w in arr {
            let name = w.get("displayName").and_then(Value::as_str).unwrap_or("").to_string();
            let type_ = w.get("type").and_then(Value::as_str).unwrap_or("").to_string();
            let balance = number_or_null(w.get("balance")).unwrap_or(0.0);
            wallets_sum += balance;
            let view = w.get("balanceView").and_then(Value::as_str).unwrap_or("").to_string();
            wallets.push(AutoClawBucket {
                bucket_id: type_.clone(),
                show_name: if name.is_empty() { type_ } else { name },
                remaining_points: balance,
                balance_view: view,
            });
        }
    }
    let available = match points.get("totalBalance") {
        Some(v) if !v.is_null() => v.clone(),
        _ if !wallets.is_empty() => json!(wallets_sum),
        _ => Value::Null,
    };
    let mut raw = Map::new();
    raw.insert("points".to_string(), points.get("raw").cloned().unwrap_or(Value::Null));
    if !expiring.is_null() {
        raw.insert("expiring".to_string(), expiring.clone());
    }
    let sub_err_text = subscription_error.as_str().unwrap_or("").to_string();
    if !sub_err_text.is_empty() {
        raw.insert("subscriptionError".to_string(), Value::String(sub_err_text));
    }
    Ok(AutoClawBalanceResult {
        available,
        unit: "Điểm".to_string(),
        wallets,
        subscription,
        expiring,
        raw: Value::Object(raw),
        user_id: credentials.user_id.clone(),
        email: credentials.email.clone(),
    })
}
