//! ZCode balance/quota query. Ported from Agent2API providers/zcode/balance.rs + router zstart.js
//! GET https://zcode.z.ai/api/v1/zcode-plan/billing/balance?app_version=3.14.0

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalanceBucket {
    pub bucket_id: String,
    pub show_name: String,
    pub meter: String,
    pub unit_type: String,
    pub total_units: i64,
    pub used_units: i64,
    pub reserved_units: i64,
    pub remaining_units: i64,
    pub available_units: i64,
    pub period_start: Option<i64>,
    pub period_end: Option<i64>,
    pub expires_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalancePlan {
    pub plan_id: String,
    pub name: String,
    pub status: String, // active | pending | expired | unknown (by time, not upstream status)
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub pending_until: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalanceResult {
    pub server_time: Option<i64>,
    pub plans: Vec<BalancePlan>,
    pub balances: Vec<BalanceBucket>,
    pub total_remaining: i64,
    pub total_used: i64,
    pub total_granted: i64,
    pub raw: Value,
}

fn secs(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn plan_state(plan: &Value, now: i64) -> String {
    let ends = plan.get("ends_at").and_then(secs).unwrap_or(0);
    if ends > 0 && ends <= now { return "expired".into(); }
    // pending if all entitlements effective_at > now
    if let Some(ents) = plan.get("entitlements").and_then(Value::as_array) {
        let all_pending = !ents.is_empty() && ents.iter().all(|e| {
            let eff = e.get("effective_at").and_then(secs).unwrap_or(0);
            eff > now
        });
        if all_pending { return "pending".into(); }
    }
    match plan.get("status").and_then(Value::as_str) {
        Some("active") => "active".into(),
        _ => "unknown".into(),
    }
}

pub async fn query_balance(jwt: &str, device_mid: &str) -> Result<BalanceResult, String> {
    if jwt.trim().is_empty() { return Err("JWT required".into()); }
    let url = format!("https://zcode.z.ai/api/v1/zcode-plan/billing/balance?app_version={}", crate::claim::app_version());
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    let mut req = client.get(&url).header("Authorization", format!("Bearer {}", jwt.trim())).header("Accept", "application/json").header("User-Agent", format!("ZCode/{}", crate::claim::app_version()));
    if !device_mid.trim().is_empty() { req = req.header("X-Device-Mid", device_mid.trim()); }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if resp.status().as_u16() == 401 { return Err("Phien dang nhap het han (401)".into()); }
    let payload: Value = resp.json().await.map_err(|e| e.to_string())?;
    let data = payload.get("data").unwrap_or(&payload);
    let now = data.get("server_time").and_then(secs).unwrap_or_else(now_unix);
    let plans: Vec<BalancePlan> = data.get("plans").and_then(Value::as_array).map(|a| a.iter().filter_map(|p| {
        Some(BalancePlan{
            plan_id: p.get("plan_id").and_then(Value::as_str).unwrap_or("").into(),
            name: p.get("name").and_then(Value::as_str).unwrap_or("").into(),
            status: plan_state(p, now),
            starts_at: p.get("starts_at").and_then(secs),
            ends_at: p.get("ends_at").and_then(secs),
            pending_until: p.get("entitlements").and_then(Value::as_array).and_then(|e| e.iter().filter_map(|x| x.get("effective_at").and_then(secs)).min()),
        })
    }).collect()).unwrap_or_default();
    let balances: Vec<BalanceBucket> = data.get("balances").and_then(Value::as_array).map(|a| a.iter().filter_map(|b| {
        Some(BalanceBucket{
            bucket_id: b.get("bucket_id").and_then(Value::as_str).unwrap_or("").into(),
            show_name: b.get("show_name").and_then(Value::as_str).unwrap_or("").into(),
            meter: b.get("meter").and_then(Value::as_str).unwrap_or("").into(),
            unit_type: b.get("unit_type").and_then(Value::as_str).unwrap_or("").into(),
            total_units: b.get("total_units").and_then(Value::as_i64).unwrap_or(0),
            used_units: b.get("used_units").and_then(Value::as_i64).unwrap_or(0),
            reserved_units: b.get("reserved_units").and_then(Value::as_i64).unwrap_or(0),
            remaining_units: b.get("remaining_units").and_then(Value::as_i64).unwrap_or(0),
            available_units: b.get("available_units").and_then(Value::as_i64).unwrap_or(0),
            period_start: b.get("period_start").and_then(secs),
            period_end: b.get("period_end").and_then(secs),
            expires_at: b.get("expires_at").and_then(secs),
        })
    }).collect()).unwrap_or_default();
    let total_remaining: i64 = balances.iter().map(|b| b.remaining_units).sum();
    let total_used: i64 = balances.iter().map(|b| b.used_units).sum();
    let total_granted: i64 = balances.iter().map(|b| b.total_units).sum();
    Ok(BalanceResult{ server_time: Some(now), plans, balances, total_remaining, total_used, total_granted, raw: payload })
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
