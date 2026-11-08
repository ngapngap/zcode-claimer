//! AutoClaw daily check-in (Diem danh hang ngay nhan diem).
//!
//! Ported from Agent2API's ProvidersPlugin/Agent2Providers
//! `crates/providers/src/providers/autoclaw/checkin.rs` + `promotion.rs`
//! (desktop daily check-in banner flow, verified 2026-09-19 / 2026-10-15).
//!
//! - Check-in is NOT a dedicated endpoint: it is the generic client-task
//!   entry `POST {userapi}/autoclaw-proxy/proxy/autoclaw-task-complete`
//!   with body `{"task_id": "daily_signin"}` (siblings:
//!   `daily_inspiration_center`, `upgrade_pc_app`).
//! - Success means "points really credited this time": `reward_points > 0`
//!   or `success == true`. HTTP 200 alone means NOTHING — an already-done
//!   day returns HTTP 200 + `success: false` + `already_completed: true` +
//!   `reward_points: 0`. Upstream is idempotent per day: repeat calls do
//!   not credit twice.
//! - Promotion (Weekly Perks / login rewards): there is NO claim endpoint;
//!   the server credits them when the account is "login active". After the
//!   check-in, read today's income ledger; if today's promo has not arrived,
//!   fire one "login" call (`POST /userapi/v1/user-info`, the same call the
//!   desktop client makes on every startup), wait 1.5s, re-check. A failure
//!   anywhere in the promotion path NEVER fails the check-in itself — the
//!   reward is extra information, not the result.
//! - `today` is the Beijing date key ("YYYY-MM-DD") passed by the frontend
//!   (the ledger's `grant_date` is a Beijing date). No date crate is used.

use serde_json::{json, Value};

use crate::autoclaw_balance::{is_auth_expired_code, userapi_get, userapi_post};
use crate::autoclaw_credentials::AutoClawCredentials;

/// Daily check-in task id (Agent2API `DAILY_SIGNIN_TASK_ID`).
const DAILY_SIGNIN_TASK_ID: &str = "daily_signin";

/// Check-in endpoint (Agent2API `completeClientTask`).
const TASK_COMPLETE_PATH: &str = "/autoclaw-proxy/proxy/autoclaw-task-complete";

/// Read-only task list (Agent2API `getTaskList`) — only used to explain a
/// failed check-in (already done today vs. broken API).
const TASK_LIST_PATH: &str = "/autoclaw-proxy/proxy/autoclaw-task-list";

/// "Login" endpoint (same call the desktop client makes on startup).
/// The server hangs the "login active" reward hook on it.
const USER_INFO_PATH: &str = "/userapi/v1/user-info";

/// Points income ledger (credit direction only — model spend never crowds
/// the window). `flow_direction` must be `income` (`credit` / `in` are
/// 400001). Page 50 leaves room for several entries in one day.
const LEDGERS_PATH: &str = "/agent-assetmgr/api/v1/ledgers_std?asset_type=point&wallet_type=all&cursor=&page_size=50&flow_direction=income";

/// Wait after the login trigger before re-checking the ledger (the server
/// credits within the same second; the check-in itself already covered
/// part of the wait).
const PROMOTION_SETTLE_MS: u64 = 1500;

/// Ledger `metadata.event_type` prefixes of "active" rewards.
const ACTIVE_EVENT_TYPE_PREFIXES: [&str; 2] =
    ["autoclaw_user_active", "autoclaw_user_signup_day"];

/// `metadata.activity_id` prefix fallback (when event_type is missing).
const ACTIVE_ACTIVITY_PREFIXES: [&str; 3] = [
    "autoclaw_user_active_reward",
    "autoclaw_user_active_bonus",
    "autoclaw_new_user_day_1_to_7_bonus",
];

/// Daily check-in WITH promotion check (Agent2API `claim_daily_signin`).
///
/// Returns `{success, rewardPoints, alreadyCompleted, reason, promotionReward}`:
/// - `reason`: "claimed" | "already" | "not_claimed" | "failed".
/// - `success` is true ONLY when points were really credited this time.
/// - `promotionReward`: `{granted, points, grantDate, entries?}` — today's
///   activity reward status (extra info; `granted: false` when nothing arrived).
pub async fn daily_checkin(credentials: &AutoClawCredentials, today: &str) -> Result<Value, String> {
    let claim = run_daily_signin(credentials).await?;

    let mut reward = today_reward(credentials, today).await.unwrap_or(Value::Null);
    let granted = reward.get("granted").and_then(Value::as_bool) == Some(true);

    if !granted && trigger_login_active(credentials).await {
        tokio::time::sleep(std::time::Duration::from_millis(PROMOTION_SETTLE_MS)).await;
        if let Ok(retry) = today_reward(credentials, today).await {
            reward = retry;
        }
    }
    Ok(attach_today_reward(claim, reward))
}

/// The check-in itself (Agent2API `run_daily_signin`).
async fn run_daily_signin(credentials: &AutoClawCredentials) -> Result<Value, String> {
    if credentials.token.trim().is_empty() {
        return Err("Khong co token, khong the diem danh".to_string());
    }

    let body = json!({ "task_id": DAILY_SIGNIN_TASK_ID });
    let payload = userapi_post(credentials, TASK_COMPLETE_PATH, &body, "Diem danh").await?;

    // Business code: an expired login is expressed as a business code here,
    // same set as the balance query.
    if let Some(code) = payload.get("code").and_then(Value::as_i64) {
        if is_auth_expired_code(code) {
            return Err("Dang nhap het han, khong the diem danh".to_string());
        }
        if code != 0 {
            let message = payload
                .get("msg")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .unwrap_or("Diem danh that bai");
            return Ok(json!({ "success": false, "rewardPoints": 0, "alreadyCompleted": false, "reason": "failed", "code": code, "msg": message }));
        }
    }

    let data = payload.get("data").cloned().unwrap_or(Value::Null);
    let already_completed = data
        .get("already_completed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let reported_success = data.get("success").and_then(Value::as_bool).unwrap_or(false);
    let reward_points = data.get("reward_points").and_then(Value::as_i64).unwrap_or(0);

    // "Claimed this time" = reward_points > 0 OR explicit success.
    // Already-done does NOT count as success: upstream answers a repeat call
    // with success:false + already_completed:true, and counting that as a
    // success would show "check-in successful" for a day with no credit.
    let claimed = reward_points > 0 || (reported_success && !already_completed);
    if claimed {
        return Ok(json!({
            "success": true,
            "rewardPoints": reward_points,
            "alreadyCompleted": false,
            "reason": "claimed",
        }));
    }

    // Not credited: tell "already done today" apart from "upstream problem".
    // The former is a normal idempotent result; the latter gets one read-only
    // task-list lookup for attribution (and its failure is not surfaced).
    if already_completed {
        return Ok(json!({
            "success": false,
            "rewardPoints": reward_points,
            "alreadyCompleted": true,
            "reason": "already",
        }));
    }
    let mut out = json!({
        "success": false,
        "rewardPoints": reward_points,
        "alreadyCompleted": false,
        "reason": "not_claimed",
    });
    if let Some(hint) = describe_task_state(credentials).await {
        if let Some(object) = out.as_object_mut() {
            object.insert("taskHint".to_string(), Value::String(hint));
        }
    }
    Ok(out)
}

/// Read-only task list lookup for attribution when a check-in was not
/// credited (Agent2API `describe_task_state`). Returns None when the list
/// carries no usable information — the caller then falls back to generic text.
async fn describe_task_state(credentials: &AutoClawCredentials) -> Option<String> {
    let payload = userapi_get(credentials, TASK_LIST_PATH, "Danh sach nhiem vu").await.ok()?;
    if payload.get("code").and_then(Value::as_i64).unwrap_or(0) != 0 {
        return None;
    }
    let tasks = payload.get("data").and_then(Value::as_array)?;
    let task = tasks
        .iter()
        .find(|item| item.get("task_id").and_then(Value::as_str) == Some(DAILY_SIGNIN_TASK_ID))?;
    let completed = task.get("completed").and_then(Value::as_bool).unwrap_or(false);
    if completed {
        Some("danh sach nhiem vu hien thi hom nay da hoan thanh".to_string())
    } else {
        None
    }
}

/// Fire one "login active" trigger — POST user-info (Agent2API
/// `trigger_login_active`). Only needs the access token (no refresh), so no
/// token rotation side effects. Returns whether the call went out; a failure
/// returns false — the promo reward is extra, it must never turn a real
/// check-in into a failure.
async fn trigger_login_active(credentials: &AutoClawCredentials) -> bool {
    if credentials.token.trim().is_empty() {
        return false;
    }
    // Same body as the desktop client's `withWebInfo` call.
    let body = json!({ "source_id": "autoclaw", "device_id": credentials.device_id });
    userapi_post(credentials, USER_INFO_PATH, &body, "Dang nhap").await.is_ok()
}

/// Today's activity reward status (Agent2API `promotion::today_reward`).
///
/// `granted: true` => `points` is the SUM of all of today's activity credits
/// (e.g. one-time bonus + daily reward), `entries` gives the per-item detail.
/// `granted: false` => nothing arrived today yet. A lookup failure is
/// returned as Err so the caller can fall back to Null (never fail check-in).
async fn today_reward(credentials: &AutoClawCredentials, today: &str) -> Result<Value, String> {
    let payload = userapi_get(credentials, LEDGERS_PATH, "Kiem tra qua hoat dong").await?;
    let entries: Vec<&Value> = payload
        .get("data")
        .and_then(|data| data.get("entries"))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter(|entry| is_today_reward(entry, today)).collect())
        .unwrap_or_default();

    let Some(first) = entries.first() else {
        return Ok(json!({ "granted": false, "grantDate": today }));
    };

    let total: i64 = entries
        .iter()
        .filter_map(|entry| entry.get("amount").and_then(Value::as_i64))
        .sum();
    let detail: Vec<Value> = entries.iter().map(|entry| entry_detail(entry)).collect();

    Ok(json!({
        "granted": true,
        "points": total,
        "description": first.get("description").and_then(Value::as_str).unwrap_or(""),
        "grantDate": today,
        "entries": detail,
    }))
}

/// One activity credit line of `entries`.
fn entry_detail(entry: &Value) -> Value {
    let metadata = entry.get("metadata");
    json!({
        "points": entry.get("amount").and_then(Value::as_i64).unwrap_or(0),
        "description": entry.get("description").and_then(Value::as_str).unwrap_or(""),
        "activityId": metadata
            .and_then(|metadata| metadata.get("activity_id"))
            .and_then(Value::as_str)
            .unwrap_or(""),
    })
}

/// Is this ledger line one of today's activity rewards?
///
/// Criterion: `metadata.grant_date` must be today, and `event_type` must hit
/// a prefix (activity_id prefix as fallback when event_type is missing).
fn is_today_reward(entry: &Value, today: &str) -> bool {
    let Some(metadata) = entry.get("metadata") else {
        return false;
    };
    let grant_date = metadata.get("grant_date").and_then(Value::as_str).unwrap_or("");
    if grant_date != today {
        return false;
    }
    let event_type = metadata.get("event_type").and_then(Value::as_str).unwrap_or("");
    if ACTIVE_EVENT_TYPE_PREFIXES
        .iter()
        .any(|prefix| event_type.starts_with(prefix))
    {
        return true;
    }
    let activity_id = metadata.get("activity_id").and_then(Value::as_str).unwrap_or("");
    ACTIVE_ACTIVITY_PREFIXES
        .iter()
        .any(|prefix| activity_id.starts_with(prefix))
}

/// Attach today's activity reward status to the check-in result
/// (Agent2API `attach_today_reward`). A lookup failure only yields Null —
/// it never upgrades into a check-in failure.
fn attach_today_reward(mut claim: Value, reward: Value) -> Value {
    if let Some(object) = claim.as_object_mut() {
        object.insert("promotionReward".to_string(), reward);
    }
    claim
}
