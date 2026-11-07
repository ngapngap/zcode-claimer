//! ZCode claim: preview + claim.
//!
//! Removed: plugin_host, routing, tracing, 27 other providers, Images API.
//! Kept: preview/claim 2 endpoints, X-Device-Mid, captcha headers, error codes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_APP_VERSION: &str = "3.14.0";
const PREVIEW_PATH: &str = "/api/v1/zcode-plan/billing/preview";
const CLAIM_PATH: &str = "/api/v1/zcode-plan/billing/claim";
const ZCODE_ORIGIN: &str = "https://zcode.z.ai";

pub fn app_version() -> String {
    std::env::var("ZCODE_APP_VERSION")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_APP_VERSION.to_string())
}

pub fn platform() -> &'static str {
    "win32-x64"
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimablePlan {
    pub plan_id: String,
    pub name: String,
    pub description: String,
    pub priority: i64,
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub entitlements: Vec<PlanEntitlement>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanEntitlement {
    pub show_name: String,
    pub unit_type: String,
    pub grant_units: i64,
    pub period: String,
    pub effective_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PreviewOutcome {
    Plans(Vec<ClaimablePlan>),
    NotDeployed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ClaimFailure {
    NotFound,
    Unavailable,
    AlreadyClaimed,
    Ineligible,
    QuotaExhausted,
    InvalidRequest,
    Captcha,
    LoginRequired,
    HttpError,
    Unknown,
}

impl ClaimFailure {
    /// Nhan tieng Viet hien cho user.
    pub fn label(self) -> &'static str {
        match self {
            Self::NotFound => "Goi khong ton tai",
            Self::Unavailable => "Hoat dong da ket thuc hoac chua bat dau",
            Self::AlreadyClaimed => "Tai khoan nay da nhan roi",
            Self::Ineligible => "Tai khoan hoac phien ban client khong du dieu kien",
            Self::QuotaExhausted => "Han muc nhan trong ngay da het",
            Self::InvalidRequest => "Tham so loi (thuong do thieu dinh danh thiet bi)",
            Self::Captcha => "Xac thuc captcha khong dat",
            Self::LoginRequired => "Phien dang nhap het han, xin dang nhap lai",
            Self::HttpError => "Loi HTTP tu upstream",
            Self::Unknown => "That bai khong ro nguyen nhan",
        }
    }

    /// Retry action cho frontend:
    /// - QuotaExhausted (1005) -> "wait_failure_ends_at": doi den failure_ends_at roi claim lai
    /// - Captcha (3007) -> "retry_captcha": lam lai captcha (slider/mint)
    /// - LoginRequired (401) -> "relogin": dang nhap lai lay JWT moi
    /// - AlreadyClaimed (1003) -> "done": coi nhu thanh cong (da nhan hom nay)
    /// - con lai -> "none"
    pub fn retry_action(self) -> &'static str {
        match self {
            Self::QuotaExhausted => "wait_failure_ends_at",
            Self::Captcha => "retry_captcha",
            Self::LoginRequired => "relogin",
            Self::AlreadyClaimed => "done",
            _ => "none",
        }
    }
    fn from_code(code: i64) -> Self {
        match code {
            1001 => Self::NotFound,
            1002 => Self::Unavailable,
            1003 => Self::AlreadyClaimed,
            1004 => Self::Ineligible,
            1005 => Self::QuotaExhausted,
            3001 => Self::InvalidRequest,
            3007 => Self::Captcha,
            401 => Self::LoginRequired,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClaimOutcome {
    Claimed {
        plan_id: String,
        starts_at: Option<i64>,
        ends_at: Option<i64>,
    },
    Failed {
        plan_id: String,
        failure: ClaimFailure,
        code: i64,
        message: String,
        failure_ends_at: Option<i64>,
    },
}

fn unix_seconds_of(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn parse_plan(raw: &Value) -> Option<ClaimablePlan> {
    let plan_id = raw.get("plan_id").and_then(Value::as_str)?.trim().to_string();
    if plan_id.is_empty() { return None; }
    let name = raw.get("name").and_then(Value::as_str).map(str::trim)
        .filter(|v| !v.is_empty()).unwrap_or(&plan_id).to_string();
    let entitlements = raw.get("entitlements").and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|item| {
            let show_name = item.get("show_name").and_then(Value::as_str)
                .map(str::trim).filter(|v| !v.is_empty())?.to_string();
            Some(PlanEntitlement {
                show_name,
                unit_type: item.get("unit_type").and_then(Value::as_str).unwrap_or("").trim().to_string(),
                grant_units: item.get("grant_units").and_then(Value::as_i64).unwrap_or(0),
                period: item.get("period").and_then(Value::as_str).unwrap_or("").trim().to_string(),
                effective_at: item.get("effective_at").and_then(unix_seconds_of),
            })
        }).collect())
        .unwrap_or_default();
    Some(ClaimablePlan {
        plan_id, name,
        description: raw.get("description").and_then(Value::as_str).unwrap_or("").trim().to_string(),
        priority: raw.get("priority").and_then(Value::as_i64).unwrap_or(0),
        starts_at: raw.get("starts_at").and_then(unix_seconds_of),
        ends_at: raw.get("ends_at").and_then(unix_seconds_of),
        entitlements,
    })
}

/// Cat chuoi theo so ky tu (khong cat giua ky tu UTF-8) de kem body server vao loi.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "..."
    }
}

pub async fn preview(jwt: &str, device_mid: &str) -> Result<PreviewOutcome, String> {
    let url = format!("{ZCODE_ORIGIN}{PREVIEW_PATH}?app_version={}&platform={}",
        urlencode(&app_version()),
        urlencode(platform()));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build().map_err(|e| e.to_string())?;
    let mut req = client.get(&url);
    if !jwt.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", jwt.trim()));
    }
    if !device_mid.trim().is_empty() {
        req = req.header("X-Device-Mid", device_mid.trim());
    }
    let resp = req.send().await
        .map_err(|e| format!("preview: loi ket noi toi server: {}", e))?;
    let status = resp.status().as_u16();
    if status == 404 { return Ok(PreviewOutcome::NotDeployed); }
    if status == 401 { return Err("Phien dang nhap het han (HTTP 401)".to_string()); }
    let body = resp.text().await
        .map_err(|e| format!("preview: doc phan hoi that bai (HTTP {}): {}", status, e))?;
    if !(200..300).contains(&status) {
        return Err(format!(
            "preview that bai: server tra HTTP {} | body: {}",
            status,
            truncate_chars(body.trim(), 300)
        ));
    }
    let payload: Value = serde_json::from_str(&body).map_err(|e| {
        format!(
            "preview: server tra ve khong phai JSON (HTTP {}): {} | body: {}",
            status,
            e,
            truncate_chars(body.trim(), 300)
        )
    })?;
    let code = payload.get("code").and_then(Value::as_i64).unwrap_or(0);
    let data = payload.get("data");
    if code != 0 {
        return Err(format!(
            "preview that bai: server tra code {} (HTTP {}) — {} | body: {}",
            code,
            status,
            error_msg(&payload),
            truncate_chars(body.trim(), 300)
        ));
    }
    if data.is_none() {
        return Err(format!(
            "preview: server tra code 0 nhung khong co data (HTTP {}) — co the chua co goi nao de preview | body: {}",
            status,
            truncate_chars(body.trim(), 300)
        ));
    }
    let plans = data.and_then(|v| v.get("plans")).and_then(Value::as_array)
        .map(|items| items.iter().filter_map(parse_plan).collect())
        .unwrap_or_default();
    Ok(PreviewOutcome::Plans(plans))
}

pub async fn claim(
    jwt: &str, device_mid: &str, plan_id: &str,
    captcha_verify_param: &str, captcha_region: &str,
) -> Result<ClaimOutcome, String> {
    if jwt.trim().is_empty() {
        return Ok(ClaimOutcome::Failed {
            plan_id: plan_id.to_string(),
            failure: ClaimFailure::LoginRequired,
            code: 401,
            message: "Nhan goi can dang nhap".to_string(),
            failure_ends_at: None,
        });
    }
    let url = format!("{ZCODE_ORIGIN}{CLAIM_PATH}");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build().map_err(|e| e.to_string())?;
    let mut req = client.post(&url)
        .header("Authorization", format!("Bearer {}", jwt.trim()))
        .header("Content-Type", "application/json")
        .header("X-ZCode-App-Version", app_version())
        .header("X-Platform", platform());
    if !captcha_verify_param.trim().is_empty() {
        req = req.header("X-Aliyun-Captcha-Verify-Param", captcha_verify_param.trim());
    }
    if !captcha_region.trim().is_empty() {
        req = req.header("X-Aliyun-Captcha-Verify-Region", captcha_region.trim());
    }
    if !device_mid.trim().is_empty() {
        req = req.header("X-Device-Mid", device_mid.trim());
    }
    let resp = req.json(&serde_json::json!({ "plan_id": plan_id }))
        .send().await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let payload: Value = resp.json().await.unwrap_or(Value::Null);
    let biz_code = payload.get("code").and_then(Value::as_i64);
    let plan = payload.get("data").and_then(|v| v.get("plan"));
    if (200..300).contains(&status) && biz_code == Some(0) {
        if let Some(p) = plan {
            return Ok(ClaimOutcome::Claimed {
                plan_id: plan_id.to_string(),
                starts_at: p.get("starts_at").and_then(Value::as_i64),
                ends_at: p.get("ends_at").and_then(Value::as_i64),
            });
        }
    }
    let failure = match biz_code {
        Some(c) => ClaimFailure::from_code(c),
        None if status == 401 => ClaimFailure::LoginRequired,
        None => ClaimFailure::HttpError,
    };
    // Khong co biz code thi kem ca HTTP status vao message de UI bao loi cu the.
    let message = match biz_code {
        Some(_) => error_msg(&payload),
        None => format!("HTTP {}: {}", status, error_msg(&payload)),
    };
    Ok(ClaimOutcome::Failed {
        plan_id: plan_id.to_string(),
        failure,
        code: biz_code.unwrap_or(i64::from(status)),
        message,
        failure_ends_at: plan.and_then(|v| v.get("ends_at")).and_then(Value::as_i64),
    })
}

fn error_msg(payload: &Value) -> String {
    for k in ["msg", "message"] {
        if let Some(t) = payload.get(k).and_then(Value::as_str) {
            let t = t.trim();
            if !t.is_empty() { return t.to_string(); }
        }
    }
    "khong co thong bao loi tu server".to_string()
}

pub fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') { out.push(c); }
        else { out.push_str(&format!("%{b:02X}")); }
    }
    out
}
