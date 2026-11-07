use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptchaConfig {
    pub enabled: bool,
    pub prefix: String,
    pub scene_id: String,
    pub region: String,
    pub skip_model_request: bool,
}

pub async fn fetch() -> Result<Option<CaptchaConfig>, String> {
    let url = format!(
        "https://zcode.z.ai/api/v1/client/configs?app_version={}&platform=win32-x64",
        crate::claim::app_version()
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build().map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let payload: Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(parse(&payload))
}

fn parse(payload: &Value) -> Option<CaptchaConfig> {
    let captcha = payload.get("data")?.get("configs")?.get("captcha")?;
    let text = |k: &str| captcha.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    let prefix = text("prefix");
    let scene_id = text("sceneId");
    if prefix.is_empty() || scene_id.is_empty() { return None; }
    Some(CaptchaConfig {
        enabled: captcha.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        prefix, scene_id,
        region: { let r = text("region"); if r.is_empty() { "ga".to_string() } else { r } },
        skip_model_request: captcha.get("skip_model_request").and_then(Value::as_bool).unwrap_or(false),
    })
}
