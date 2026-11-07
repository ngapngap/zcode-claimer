//! Activation report (app_launch / app_daily_active).
//! Must be called BEFORE preview — upstream rejects probe without it.

use serde_json::{json, Value};

pub async fn report(jwt: &str, device_mid: &str) -> Result<(), String> {
    if jwt.trim().is_empty() { return Err("JWT required".to_string()); }
    // Two events: app_launch + app_daily_active (both required by upstream)
    for event in ["app_launch", "app_daily_active"] {
        let url = "https://zcode.z.ai/api/v1/client/activation";
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build().map_err(|e| e.to_string())?;
        let body = json!({
            "event": event,
            "device_id": device_mid,
            "device_os_category": "windows",
            "app_version": crate::claim::app_version(),
        });
        let _ = client.post(url)
            .header("Authorization", format!("Bearer {}", jwt.trim()))
            .header("X-Device-Mid", device_mid.trim())
            .header("Content-Type", "application/json")
            .json(&body)
            .send().await;
        // Best-effort: don't fail whole flow if activation event fails
    }
    Ok(())
}
