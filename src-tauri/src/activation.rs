//! Activation report (app_launch / app_daily_active).
//!
//! Eligibility gate, not telemetry: upstream only lists claimable plans
//! (preview) for accounts that have reported being active today.
//! Official client fires both events on launch / daily-active; this
//! backend must do the same before preview, or preview returns empty
//! (`code 0` + no `data`) with no error — exactly the failure reported
//! in m02612, while Agent2API claims fine because it reports first.
//
//! Endpoints (both regions share the domain):
//!   POST https://zcode.z.ai/api/v1/event/report
//! Body carries identity (`user_id` + `device_mid`) — NO Authorization
//! header. Each event has its own UUID `event_id` (upstream dedupes).
//! Envelope is `{code,msg}`; success is `code == 0`.

use serde_json::{json, Value};

const EVENT_REPORT_URL: &str = "https://zcode.z.ai/api/v1/event/report";
const ACTIVATION_EVENTS: [&str; 2] = ["app_launch", "app_daily_active"];
const REQUEST_TIMEOUT_MS: u64 = 10_000;
const CLIENT_LANGUAGE: &str = "zh-CN";
const SCREEN_RESOLUTION: &str = "2560x1440";

fn envelope_message(payload: &Value, code: i64) -> String {
    payload
        .get("msg")
        .or_else(|| payload.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("business code {code}"))
}

/// Report activation for one account: fires both events, returns the
/// first failure reason. Empty `user_id` means the account has no
/// upstream identity (manual API-key accounts) — skip uploading,
/// like Agent2API `report_activation` returns `(None, None)`.
pub async fn report(user_id: &str, device_mid: &str) -> Result<(), String> {
    let user_id = user_id.trim();
    if user_id.is_empty() {
        return Err(
            "Tai khoan nay khong co userId, khong the bao cao kich hoat (can dang nhap bang OAuth)"
                .to_string(),
        );
    }
    let device_mid = device_mid.trim();
    // Timezone / OS version are environment probes; the official client's
    // timezone is mapped to IANA. Unrecognised -> "unknown" (same fallback).
    let timezone = probe_timezone();
    let os_version = probe_os_version();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(REQUEST_TIMEOUT_MS))
        .build()
        .map_err(|e| e.to_string())?;

    let mut first_error: Option<String> = None;
    for event in ACTIVATION_EVENTS {
        let Some(event_id) = new_uuid() else {
            let reason = "Khong the sinh event id (nguon random khong kha dung)".to_string();
            return Err(first_error.unwrap_or(reason));
        };
        let body = json!({
            "event_id": event_id,
            "client_timezone": timezone,
            "client_language": CLIENT_LANGUAGE,
            "element_name": event,
            "event_region": "app",
            "event_type": "view",
            "event_text": "",
            "event_extra_detail": {},
            "user_id": user_id,
            "screen_resolution": SCREEN_RESOLUTION,
            "app_version": crate::claim::app_version(),
            "device_os_category": os_category(),
            "device_os_version": os_version,
            "device_mid": device_mid,
            "mac_id": "",
            "marketing_params": "{}",
        });
        let failure = match client
            .post(EVENT_REPORT_URL)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => {
                let payload: Value = resp.json().await.unwrap_or(Value::Null);
                let code = payload.get("code").and_then(Value::as_i64).unwrap_or(-1);
                (code != 0).then(|| {
                    format!("Activation event rejected by upstream: {}", envelope_message(&payload, code))
                })
            }
            Err(e) => Some(if e.is_timeout() {
                "Activation event upload timed out".to_string()
            } else {
                format!("Activation event upload failed: {e}")
            }),
        };
        if let Some(reason) = failure {
            if first_error.is_none() {
                first_error = Some(reason);
            }
        }
    }
    match first_error {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

fn new_uuid() -> Option<String> {
    let id = uuid::Uuid::new_v4();
    if id.is_nil() {
        return None;
    }
    Some(id.to_string())
}

/// Host timezone as IANA name. Only Windows has a real probe
/// (`tzutil /g`); everything else falls back to "unknown".
fn probe_timezone() -> String {
    #[cfg(windows)]
    {
        use std::process::Command;
        let output = Command::new("tzutil").arg("/g").output().ok();
        let name = output
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .unwrap_or_default();
        match name.as_str() {
            "China Standard Time" | "China Daylight Time" => "Asia/Shanghai",
            "Singapore Standard Time" => "Asia/Singapore",
            "Tokyo Standard Time" => "Asia/Tokyo",
            "UTC" => "UTC",
            _ => "unknown",
        }
        .to_string()
    }
    #[cfg(not(windows))]
    {
        "unknown".to_string()
    }
}

/// Host OS version as "10.0.{build}" on Windows; "" elsewhere.
fn probe_os_version() -> String {
    #[cfg(windows)]
    {
        use std::process::Command;
        let output = Command::new("reg")
            .args([
                "query",
                r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                "/v",
                "CurrentBuildNumber",
            ])
            .output()
            .ok();
        let text = output
            .map(|out| String::from_utf8_lossy(&out.stdout).to_string())
            .unwrap_or_default();
        text.lines()
            .find(|line| line.contains("CurrentBuildNumber"))
            .and_then(|line| line.split_whitespace().last())
            .map(|build| format!("10.0.{build}"))
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}

fn os_category() -> &'static str {
    #[cfg(windows)]
    {
        "windows"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        "linux"
    }
}
