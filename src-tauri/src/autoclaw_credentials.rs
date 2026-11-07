//! AutoClaw credential sources (ported from AutoClaw desktop protocol;
//! env override + AES-key cache restored).
//!
//! Priority: 1) %APPDATA%/AutoClaw/auth.json (DPAPI-decrypted token/refreshToken),
//! 2) ~/.openclaw-autoclaw/openclaw.json plaintext X-Authorization JWT (no refresh).
//! Region is ALWAYS caller-supplied (Intl is default for this claimer); the file
//! itself carries no region marker so we never guess.
//!
//! Units: AutoClaw balances are POINTS (diem), not tokens.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Map, Value};

use crate::autoclaw_crypto;

pub const DESKTOP_ACCOUNT_ID: &str = "desktop-auth";
const MAX_AUTH_FILE_SIZE: u64 = 256 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Region {
    #[default]
    Cn,
    Intl,
}

impl Region {
    pub fn userapi_base_url(&self) -> &'static str {
        match self {
            Self::Cn => "https://autoglm-acceleration-api.zhipuai.cn",
            Self::Intl => "https://autoglm-api.autoglm.ai",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Cn => "Cn",
            Self::Intl => "Intl",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AutoClawCredentials {
    pub id: String,
    pub region: Region,
    pub token: String,
    pub refresh_token: String,
    pub device_id: String,
    pub user_id: String,
    pub email: String,
    pub expires_at: Option<f64>,
}

fn home_dir() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn default_user_data_dir() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("AUTOCLAW_USER_DATA_DIR") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    if !cfg!(windows) {
        return None;
    }
    std::env::var("APPDATA")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map(|appdata| appdata.join("AutoClaw"))
        .or_else(|| Some(home_dir().join("AppData").join("Roaming").join("AutoClaw")))
}

pub fn desktop_auth_file() -> Option<PathBuf> {
    default_user_data_dir().map(|dir| dir.join("auth.json"))
}

fn local_state_file() -> Option<PathBuf> {
    default_user_data_dir().map(|dir| dir.join("Local State"))
}

pub fn gateway_config_file() -> PathBuf {
    home_dir().join(".openclaw-autoclaw").join("openclaw.json")
}

struct FileSnapshot {
    json: Map<String, Value>,
    _modified_at: i64,
}

fn read_json_file(path: &Path, label: &str) -> Result<FileSnapshot, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!("Khong thay {}: {}", label, path.display())
        } else {
            format!("Khong doc duoc {}: {}", label, error)
        }
    })?;
    if meta.file_type().is_symlink() {
        return Err(format!("{} la symlink, tu choi doc", label));
    }
    if !meta.is_file() {
        return Err(format!("{} khong phai file", label));
    }
    if meta.len() == 0 || meta.len() > MAX_AUTH_FILE_SIZE {
        return Err(format!("{} co kich thuoc bat thuong", label));
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("Khong doc duoc {}: {}", label, e))?;
    let value = serde_json::from_str::<Value>(&text)
        .map_err(|_| format!("{} khong phai JSON hop le", label))?;
    let Value::Object(json) = value else {
        return Err(format!("{} goc phai la JSON object", label));
    };
    let modified_at = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok(FileSnapshot { json, _modified_at: modified_at })
}

pub(crate) fn js_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn number_value(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    number.is_finite().then_some(number)
}

fn credentials_from_claims(
    id: String,
    region: Region,
    token: String,
    refresh_token: String,
    device_id: String,
    claims: Option<&Value>,
) -> AutoClawCredentials {
    let user_id = claims
        .and_then(|c| c.get("user_id"))
        .map(js_text)
        .unwrap_or_default();
    let expires_at = claims
        .and_then(|c| c.get("exp"))
        .and_then(number_value)
        .filter(|v| *v > 0.0)
        .map(|v| v * 1000.0);
    let device_id = if !device_id.is_empty() {
        device_id
    } else {
        claims
            .and_then(|c| c.get("device_id"))
            .map(js_text)
            .unwrap_or_default()
    };
    AutoClawCredentials {
        id,
        region,
        token,
        refresh_token,
        device_id,
        user_id,
        email: String::new(),
        expires_at,
    }
}

fn from_auth_file(region: Region) -> Result<AutoClawCredentials, String> {
    let Some(auth_path) = desktop_auth_file() else {
        return Err("AutoClaw desktop auth.json chi ho tro Windows".to_string());
    };
    let snapshot = read_json_file(&auth_path, "AutoClaw auth.json")?;
    let text_field = |key: &str| -> String {
        snapshot
            .json
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let raw_token = text_field("token");
    let raw_refresh = text_field("refreshToken");
    let device_id = text_field("deviceId").trim().to_string();
    if raw_token.is_empty() && raw_refresh.is_empty() {
        return Err("AutoClaw auth.json thieu token".to_string());
    }
    let mut key_error: Option<String> = None;
    let aes_key = match local_state_file() {
        Some(local_state) => match autoclaw_crypto::os_crypt_aes_key(&local_state) {
            Ok(key) => Some(key),
            Err(reason) => {
                key_error = Some(reason);
                None
            }
        },
        None => {
            key_error = Some("AutoClaw desktop chi ho tro Windows".to_string());
            None
        }
    };
    if aes_key.is_none() && raw_token.starts_with("enc:") {
        return Err(format!(
            "AutoClaw token ma hoa nhung giai ma that bai: {}",
            key_error.unwrap_or_else(|| "thieu os_crypt key".to_string())
        ));
    }
    let token = autoclaw_crypto::strip_bearer(&autoclaw_crypto::decrypt_enc_value(
        &raw_token,
        aes_key.as_deref(),
    )?);
    let refresh_token = autoclaw_crypto::strip_bearer(
        &autoclaw_crypto::decrypt_enc_value(&raw_refresh, aes_key.as_deref())
            .unwrap_or_default(),
    );
    if token.is_empty() {
        return Err("AutoClaw auth.json token giai ma ra rong".to_string());
    }
    let claims = autoclaw_crypto::decode_jwt_claims(&token);
    let mut credentials = credentials_from_claims(
        DESKTOP_ACCOUNT_ID.to_string(),
        region,
        token,
        refresh_token,
        device_id,
        claims.as_ref(),
    );
    credentials.email = snapshot
        .json
        .get("userInfo")
        .and_then(|info| info.get("email"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    Ok(credentials)
}

fn from_gateway_config(region: Region) -> Result<AutoClawCredentials, String> {
    let path = gateway_config_file();
    let snapshot = read_json_file(&path, "AutoClaw openclaw.json")?;
    let providers = snapshot
        .json
        .get("models")
        .and_then(|m| m.get("providers"))
        .and_then(Value::as_object);
    fn provider_score(name: &str, provider: &Value) -> i32 {
        let lower = name.to_lowercase();
        let mut score = 0;
        if lower.contains("autoclaw") || lower.contains("glm") || lower.contains("zcode") { score += 10; }
        if provider.get("baseUrl").and_then(Value::as_str).unwrap_or("").to_lowercase().contains("autoglm") { score += 5; }
        score
    }
    let mut scored: Vec<(i32, Value)> = snapshot.json
        .get("models")
        .and_then(|m| m.get("providers"))
        .and_then(Value::as_object)
        .map(|map| map.iter().map(|(k, v)| (provider_score(k, v), v.clone())).collect())
        .unwrap_or_default();
    scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    let mut token = String::new();
    'outer: for (_, provider) in scored.iter() {
        let models = match provider.get("models").and_then(Value::as_array) {
            Some(models) => models,
            None => continue,
        };
        for model in models {
            let headers = match model.get("headers").and_then(Value::as_object) {
                Some(headers) => headers,
                None => continue,
            };
            for name in ["X-Authorization", "x-authorization"] {
                if let Some(Value::String(candidate)) = headers.get(name) {
                    let normalized = autoclaw_crypto::strip_bearer(candidate);
                    if !normalized.is_empty() { token = normalized; break 'outer; }
                }
            }
        }
    }
    if token.is_empty() {
        return Err("openclaw.json thieu X-Authorization token".to_string());
    }
    let claims = autoclaw_crypto::decode_jwt_claims(&token);
    Ok(credentials_from_claims(
        DESKTOP_ACCOUNT_ID.to_string(),
        region,
        token,
        String::new(),
        String::new(),
        claims.as_ref(),
    ))
}

/// Local credentials: auth.json first, fallback openclaw.json.
/// Two regions share the same file — region is caller-supplied, never guessed.
pub fn local_credentials(region: Region) -> Result<AutoClawCredentials, String> {
    match from_auth_file(region) {
        Ok(c) => Ok(c),
        Err(auth_err) => match from_gateway_config(region) {
            Ok(c) => Ok(c),
            Err(_) => Err(format!(
                "AutoClaw desktop login khong kha dung: {}",
                auth_err
            )),
        },
    }
}

/// Env credentials: {AUTOCLAW_TOKEN} (Cn) / {AUTOCLAW_INTL_TOKEN} (Intl).
pub fn env_credentials(region: Region) -> Option<AutoClawCredentials> {
    let key = match region {
        Region::Cn => "AUTOCLAW_TOKEN",
        Region::Intl => "AUTOCLAW_INTL_TOKEN",
    };
    let token = std::env::var(key).ok()?;
    let token = autoclaw_crypto::strip_bearer(&token);
    if token.is_empty() {
        return None;
    }
    let refresh_key = match region {
        Region::Cn => "AUTOCLAW_REFRESH_TOKEN",
        Region::Intl => "AUTOCLAW_INTL_REFRESH_TOKEN",
    };
    let refresh_token = std::env::var(refresh_key)
        .ok()
        .map(|v| autoclaw_crypto::strip_bearer(&v))
        .unwrap_or_default();
    let device_key = match region {
        Region::Cn => "AUTOCLAW_DEVICE_ID",
        Region::Intl => "AUTOCLAW_INTL_DEVICE_ID",
    };
    let device_id = std::env::var(device_key).unwrap_or_default();
    let claims = autoclaw_crypto::decode_jwt_claims(&token);
    Some(credentials_from_claims(
        "env".to_string(),
        region,
        token,
        refresh_token,
        device_id,
        claims.as_ref(),
    ))
}

/// From a saved manual account record (token plain or enc:).
pub fn credentials_from_token(region: Region, token: &str, refresh_token: &str, device_id: &str) -> Result<AutoClawCredentials, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("Thieu AutoClaw token".to_string());
    }
    let mut aes_key: Option<Vec<u8>> = None;
    if token.starts_with("enc:") {
        let local_state = local_state_file()
            .ok_or_else(|| "enc: token can Windows de giai ma".to_string())?;
        aes_key = Some(autoclaw_crypto::os_crypt_aes_key(&local_state)?);
    }
    let token = autoclaw_crypto::strip_bearer(&autoclaw_crypto::decrypt_enc_value(
        &token,
        aes_key.as_deref(),
    )?);
    let refresh = autoclaw_crypto::strip_bearer(
        &autoclaw_crypto::decrypt_enc_value(refresh_token.trim(), aes_key.as_deref())
            .unwrap_or_default(),
    );
    let claims = autoclaw_crypto::decode_jwt_claims(&token);
    Ok(credentials_from_claims(
        "manual".to_string(),
        region,
        token,
        refresh,
        device_id.trim().to_string(),
        claims.as_ref(),
    ))
}

#[allow(dead_code)]
fn _unused(_m: HashMap<String, String>) {}
