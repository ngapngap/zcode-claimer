//! ZCode balance/quota query (theo protocol ZCode).
//! GET https://zcode.z.ai/api/v1/zcode-plan/billing/balance?app_version=3.14.0&platform=win32-x64
//!
//! Ngu nghia official (zai-org/ZCode zaiStartPlanBilling.ts) duoc giu nguyen:
//!   1. Trang thai goi xet theo thoi gian (ends_at da qua -> expired;
//!      entitlements[].effective_at con tuong lai -> pending), status chi dung
//!      lam du phong. effective_at = 0 la sentinel "hieu luc ngay".
//!   2. Bucket thuoc goi khong dung duoc (expired / entitlements chua toi han)
//!      bi loai **khoi tong** — goi Weekend Build/Trust Build vua nhan
//!      (effective_at 23:00 hom nay) khong duoc gia mao so du kha dung, chi
//!      hien thi o danh sach goi + huy hieu "Cho hieu luc".
//!   3. Bucket khong nhan duoc chu goi van giu lai (khong xoa cua goi hop le).
//!   4. remaining lay theo chuoi: remaining_units -> total-used -> available_units;
//!      so co the la number | string | null (ca 3 deu ton tai o upstream).
//!   5. Khong gop bucket theo show_name: moi bucket la mot dong chi tiet.
//! Chuan bi so theo server_time cua upstream, khong theo dong ho may cuc bo.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalanceBucket {
    pub bucket_id: String,
    pub user_plan_id: Option<String>,
    pub plan_id: Option<String>,
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
    /// active | pending | expired | unknown (by time, not upstream status)
    pub status: String,
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    /// Hieu luc sap toi gan nhat (unix sec); None neu khong con phai cho.
    pub pending_until: Option<i64>,
    /// Co it nhat mot entitlement chua toi han khong (de hien huy "Cho hieu luc").
    pub has_pending: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalanceResult {
    pub server_time: Option<i64>,
    pub plans: Vec<BalancePlan>,
    pub balances: Vec<BalanceBucket>,
    pub total_remaining: i64,
    pub total_used: i64,
    pub total_granted: i64,
    pub has_quota: bool,
    pub raw: Value,
}

/// So upstream: number | string | null deu nhan (official TS: number | string | null).
fn num(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_f64().map(|f| f as i64),
        Value::String(s) => s.trim().parse::<f64>().ok().map(|f| f as i64),
        _ => None,
    }
}

/// Thoi gian tu upstream, quy doi ve unix sec (lon hon 1e11 coi la ms).
fn secs(v: &Value) -> Option<i64> {
    num(v).map(|n| if n > 100_000_000_000 { n / 1000 } else { n })
}

/// Hieu luc goc tu entitlements[].effective_at (>0), goi khong co entitlement
/// moi dung starts_at du phong. 0 = sentinel hieu luc ngay (bo qua).
fn effective_times(plan: &Value) -> Vec<i64> {
    if let Some(items) = plan.get("entitlements").and_then(Value::as_array) {
        if !items.is_empty() {
            return items
                .iter()
                .filter_map(|e| e.get("effective_at").and_then(num))
                .filter(|t| *t > 0)
                .collect();
        }
    }
    plan.get("starts_at")
        .and_then(secs)
        .filter(|t| *t > 0)
        .into_iter()
        .collect()
}

/// Hieu luc sap toi gan nhat; None neu khong con entitlement nao chua toi han.
fn pending_until(plan: &Value, now: i64) -> Option<i64> {
    effective_times(plan)
        .into_iter()
        .filter(|t| *t > now)
        .min()
}

/// Moi entitlement deu chua toi han (official availability: every(v > now)).
fn all_pending(plan: &Value, now: i64) -> bool {
    let times = effective_times(plan);
    !times.is_empty() && times.iter().all(|t| *t > now)
}

/// Trang thai goi: thoi gian uu tien, status chi du phong.
fn plan_state(plan: &Value, now: i64) -> String {
    if let Some(ends) = plan.get("ends_at").and_then(secs).filter(|t| *t > 0) {
        if ends <= now {
            return "expired".into();
        }
    }
    if pending_until(plan, now).is_some() {
        return "pending".into();
    }
    match plan.get("status").and_then(Value::as_str).map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("active") | Some("valid") => "active".into(),
        Some(s) if s.contains("expire") || s.contains("cancel") || s == "ended" || s == "invalid" => "expired".into(),
        _ => "unknown".into(),
    }
}

/// Goi con dung duoc khong: chua expired va khong phai moi entitlement deu
/// chua toi han. Status la khong nhan duoc van duoc kke — chung la gioi han
/// nhan dien cua minh, khong duoc khien so du bien mat.
fn usable(plan: &Value, now: i64) -> bool {
    plan_state(plan, now) != "expired" && !all_pending(plan, now)
}

pub async fn query_balance(jwt: &str, device_mid: &str) -> Result<BalanceResult, String> {
    if jwt.trim().is_empty() {
        return Err("JWT required".into());
    }
    let url = format!(
        "https://zcode.z.ai/api/v1/zcode-plan/billing/balance?app_version={}&platform={}",
        crate::claim::app_version(),
        crate::claim::platform()
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", jwt.trim()))
        .header("Accept", "application/json");
    if !device_mid.trim().is_empty() {
        req = req.header("X-Device-Mid", device_mid.trim());
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let payload: Value = resp.json().await.unwrap_or(Value::Null);
    let code = payload.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
    let message = payload
        .get("msg")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if status == 401 || code == 401 {
        return Err("Phien dang nhap het han (401) — hay dang nhap lai".into());
    }
    if code == 3001 {
        return Err("Bi tu choi tham so (3001: thieu/sai deviceMid) — hay dang nhap lai".into());
    }
    if status == 429 {
        return Err("Bi gioi han toc do (429) — doi mot chut rui thu lai".into());
    }
    if !(200..300).contains(&status) || code != 0 {
        let detail = if message.is_empty() { format!("HTTP {}", status) } else { message };
        return Err(format!("Query so du that bai: {}", detail));
    }
    let data = payload.get("data").cloned().unwrap_or_else(|| json!({}));
    Ok(normalize(&data, payload))
}

fn normalize(data: &Value, raw: Value) -> BalanceResult {
    let now = data.get("server_time").and_then(secs).filter(|t| *t > 0).unwrap_or_else(now_unix);
    let plans_raw: Vec<&Value> = data.get("plans").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();

    let plans: Vec<BalancePlan> = plans_raw
        .iter()
        .map(|p| BalancePlan {
            plan_id: p.get("plan_id").and_then(Value::as_str).unwrap_or("").into(),
            name: p.get("name").and_then(Value::as_str).unwrap_or("").into(),
            status: plan_state(p, now),
            starts_at: p.get("starts_at").and_then(secs),
            ends_at: p.get("ends_at").and_then(secs),
            pending_until: pending_until(p, now),
            has_pending: pending_until(p, now).is_some(),
        })
        .collect();

    // Chu goi: user_plan_id uu tien, phai loai moi tru khi moi goi deu khong dung duoc.
    let owner_unusable = |b: &Value| -> bool {
        let user_plan_id = b.get("user_plan_id").and_then(Value::as_str);
        let plan_id = b.get("plan_id").and_then(Value::as_str);
        let owners: Vec<&&Value> = plans_raw
            .iter()
            .filter(|plan| match (user_plan_id, plan.get("user_plan_id").and_then(Value::as_str)) {
                (Some(l), Some(r)) => l == r,
                _ => plan_id.is_some() && plan.get("plan_id").and_then(Value::as_str) == plan_id,
            })
            .collect();
        !owners.is_empty() && owners.iter().all(|plan| !usable(plan, now))
    };

    let mut balances: Vec<BalanceBucket> = Vec::new();
    if let Some(items) = data.get("balances").and_then(Value::as_array) {
        for b in items.iter().filter(|b| !owner_unusable(b)) {
            let total = b.get("total_units").and_then(num).unwrap_or(0);
            let used = b.get("used_units").and_then(num).unwrap_or(0);
            // Chuoi remaining: remaining_units -> total-used -> available_units
            let remaining = b
                .get("remaining_units")
                .and_then(num)
                .or_else(|| match (b.get("total_units").and_then(num), b.get("used_units").and_then(num)) {
                    (Some(t), Some(u)) => Some(t - u),
                    _ => None,
                })
                .or_else(|| b.get("available_units").and_then(num));
            let Some(remaining) = remaining else { continue };
            let available = b.get("available_units").and_then(num).unwrap_or(remaining);
            balances.push(BalanceBucket {
                bucket_id: b.get("bucket_id").and_then(Value::as_str).unwrap_or("").into(),
                user_plan_id: b.get("user_plan_id").and_then(Value::as_str).map(str::to_string),
                plan_id: b.get("plan_id").and_then(Value::as_str).map(str::to_string),
                show_name: b.get("show_name").and_then(Value::as_str).unwrap_or("").into(),
                meter: b.get("meter").and_then(Value::as_str).unwrap_or("").into(),
                unit_type: b.get("unit_type").and_then(Value::as_str).unwrap_or("").into(),
                total_units: total,
                used_units: used,
                reserved_units: b.get("reserved_units").and_then(num).unwrap_or(0),
                remaining_units: remaining,
                available_units: available,
                period_start: b.get("period_start").and_then(secs),
                period_end: b.get("period_end").and_then(secs),
                expires_at: b.get("expires_at").and_then(secs),
            });
        }
    }
    let total_remaining: i64 = balances.iter().map(|b| b.remaining_units).sum();
    let total_used: i64 = balances.iter().map(|b| b.used_units).sum();
    let total_granted: i64 = balances.iter().map(|b| b.total_units).sum();
    let has_quota = !balances.is_empty();
    BalanceResult { server_time: Some(now), plans, balances, total_remaining, total_used, total_granted, has_quota, raw }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
