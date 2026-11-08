//! Persistence: config local (JWT + device_mid + account fields) va claim history.
//! File nam trong app-data cua Tauri (khong commit git). Khong log JWT.
//! Retry semantics:
//! - 1005 (QuotaExhausted) -> UI doi den `failure_ends_at` roi claim lai
//! - 3007 (Captcha) -> can lam lai captcha (slider / traceless mint)
//! - 401 (LoginRequired) -> can login lai lay JWT moi
//! Cac han so nay duoc expose qua `ClaimFailure::action` trong claim.rs de frontend dung.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const CONFIG_FILE: &str = "zcode-claimer-config.json";
const HISTORY_FILE: &str = "claim-history.json";
const HISTORY_MAX: usize = 200;

/// Multi-account: moi tai khoan (key=user_id) co SavedConfig rieng, khong de ghi de.
/// File: accounts.json  { "user_id": SavedConfig, ... }
/// Config cu single-account van doc duoc de migrate (neu chua co accounts.json).
const ACCOUNTS_FILE: &str = "accounts.json";

/// Cau hinh luu local: deviceMid + JWT + cac truong account de export router.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SavedConfig {
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub device_mid: String,
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub jwt: String,
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub user_id: String,
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub username: String,
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub email: String,
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub zai_business_token: String,
}

fn empty_string_if_null<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<String>::deserialize(d)?;
    Ok(v.unwrap_or_default())
}

/// Mot dong claim history: plan_id + ngay + ket qua + code.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimHistoryEntry {
    /// plan_id cua plan da claim (vi du zcode-v3-start-plan-trust-0928)
    pub plan_id: String,
    /// ngay claim YYYY-MM-DD (gio VN, frontend gui hoac backend lay tu ngay hien tai)
    pub date: String,
    /// ket qua: "claimed" | "failed"
    pub outcome: String,
    /// code nghiep vu tu upstream (0 = thanh cong), 401/1005/3007...
    pub code: i64,
    /// nhan loi tieng Viet (ClaimFailure::label) hoac message tu upstream
    pub message: String,
    /// failure_ends_at (unix seconds) khi code 1005 — UI doi den moc nay
    pub failure_ends_at: Option<i64>,
    /// thoi diem ghi lich su (unix seconds)
    pub recorded_at: i64,
}

fn app_data_dir() -> Result<PathBuf, String> {
    // Giai quyet them thu muc app-data cua Tauri bang cach dung bien moi truong
    // ma Tauri set khi chay; fallback ve thu muc hien tai neu khong co.
    // Encrypted / project dirs khong dung de tranh them dependency.
    if let Ok(dir) = std::env::var("ZCODE_CLAIMER_DATA_DIR") {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            return Ok(p);
        }
    }
    Ok(PathBuf::from("."))
}

fn write_json(path: &PathBuf, value: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    // Ghi tam roi rename de tranh file do khi process kill giua chung.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn read_json(path: &PathBuf) -> Result<Option<serde_json::Value>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => {
            Ok(Some(serde_json::from_str(&text).map_err(|e| e.to_string())?))
        }
        Ok(_) => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Lay duong dan file config trong app-data; nhan app-data dir tu Tauri AppHandle
/// de dung dung thu muc Tauri quy dinh (uu tien), fallback app_data_dir().
pub fn config_path(app_data: Option<PathBuf>) -> Result<PathBuf, String> {
    let dir = app_data.unwrap_or_else(|| app_data_dir().unwrap_or_else(|_| PathBuf::from(".")));
    Ok(dir.join(CONFIG_FILE))
}

pub fn history_path(app_data: Option<PathBuf>) -> Result<PathBuf, String> {
    let dir = app_data.unwrap_or_else(|| app_data_dir().unwrap_or_else(|_| PathBuf::from(".")));
    Ok(dir.join(HISTORY_FILE))
}

/// Nap config da luu (khong ton tai thi tra Default). Khong log JWT.
pub fn load_config(app_data: Option<PathBuf>) -> Result<SavedConfig, String> {
    let path = config_path(app_data)?;
    match read_json(&path)? {
        Some(v) => serde_json::from_value(v).map_err(|e| e.to_string()),
        None => Ok(SavedConfig::default()),
    }
}

/// Ghi config xuong file local. JWT chi nam trong file nay, khong ra console/log.
pub fn save_config(app_data: Option<PathBuf>, cfg: &SavedConfig) -> Result<(), String> {
    let path = config_path(app_data)?;
    let v = serde_json::to_value(cfg).map_err(|e| e.to_string())?;
    write_json(&path, &v)
}

/// Doc claim history (moi nhat truoc). File chua co thi tra rong.
pub fn load_history(app_data: Option<PathBuf>) -> Result<Vec<ClaimHistoryEntry>, String> {
    let path = history_path(app_data)?;
    match read_json(&path)? {
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                if let Ok(e) = serde_json::from_value::<ClaimHistoryEntry>(item) {
                    out.push(e);
                }
            }
            Ok(out)
        }
        Some(_) => Ok(Vec::new()),
        None => Ok(Vec::new()),
    }
}

/// Them mot dong lich su, giu toi da HISTORY_MAX dong moi nhat, ghi xuong file.
pub fn append_history(app_data: Option<PathBuf>, entry: ClaimHistoryEntry) -> Result<Vec<ClaimHistoryEntry>, String> {
    let mut all = load_history(app_data.clone())?;
    all.insert(0, entry);
    if all.len() > HISTORY_MAX {
        all.truncate(HISTORY_MAX);
    }
    let path = history_path(app_data)?;
    let v = serde_json::to_value(&all).map_err(|e| e.to_string())?;
    write_json(&path, &v)?;
    Ok(all)
}

// ── Multi-account storage ──

const HISTORY_MAX_ACCOUNTS: usize = 200;

fn accounts_path(app_data: &Option<PathBuf>) -> Result<PathBuf, String> {
    let dir = app_data.clone().unwrap_or_else(|| app_data_dir().unwrap_or_else(|_| PathBuf::from(".")));
    Ok(dir.join(ACCOUNTS_FILE))
}

fn load_accounts_raw(app_data: &Option<PathBuf>) -> Result<std::collections::HashMap<String, SavedConfig>, String> {
    let path = accounts_path(app_data)?;
    match read_json(&path)? {
        Some(serde_json::Value::Object(map)) => {
            let mut out = std::collections::HashMap::new();
            for (k, v) in map {
                if let Ok(cfg) = serde_json::from_value::<SavedConfig>(v) {
                    out.insert(k, cfg);
                }
            }
            Ok(out)
        }
        Some(_) => Ok(std::collections::HashMap::new()),
        None => Ok(std::collections::HashMap::new()),
    }
}

fn save_accounts_raw(app_data: &Option<PathBuf>, map: &std::collections::HashMap<String, SavedConfig>) -> Result<(), String> {
    let path = accounts_path(app_data)?;
    let v = serde_json::to_value(map).map_err(|e| e.to_string())?;
    write_json(&path, &v)
}

/// Nap account theo user_id. Neu chua co accounts.json thi migrate config cu single neu khop.
pub fn load_account(app_data: Option<PathBuf>, user_id: &str) -> Result<SavedConfig, String> {
    let map = load_accounts_raw(&app_data)?;
    if let Some(cfg) = map.get(user_id) {
        return Ok(cfg.clone());
    }
    // Migrate single-account len accounts.json neu chua co (ghi 1 lan)
    let single = load_config(app_data.clone())?;
    if !single.user_id.is_empty() && (single.user_id == user_id || user_id.is_empty()) {
        let mut m = map;
        m.insert(single.user_id.clone(), single.clone());
        let _ = save_accounts_raw(&app_data, &m);
        if user_id.is_empty() {
            return Ok(single);
        }
        if let Some(c) = m.get(user_id) {
            return Ok(c.clone());
        }
    }
    Ok(SavedConfig::default())
}

/// Ghi account theo user_id (key = user_id de tranh ghi de). Neu user_id rong thi fallback single.
pub fn save_account(app_data: Option<PathBuf>, cfg: &SavedConfig) -> Result<(), String> {
    let key = cfg.user_id.trim().to_string();
    if key.is_empty() {
        return save_config(app_data, cfg);
    }
    let mut map = load_accounts_raw(&app_data)?;
    map.insert(key, cfg.clone());
    save_accounts_raw(&app_data, &map)
}

/// Xoa account theo user_id khoi accounts.json. Neu khong co trong map thi
/// xoa ca single config cu neu khop. Tra ve true neu da xoa duoc cai gi do.
pub fn delete_account(app_data: Option<PathBuf>, user_id: &str) -> Result<bool, String> {
    let key = user_id.trim();
    if key.is_empty() {
        return Ok(false);
    }
    let mut map = load_accounts_raw(&app_data)?;
    if map.remove(key).is_some() {
        save_accounts_raw(&app_data, &map)?;
        return Ok(true);
    }
    // Fallback: single config cu co the dang giu account nay.
    let single = load_config(app_data.clone())?;
    if single.user_id == key {
        save_config(app_data, &SavedConfig::default())?;
        return Ok(true);
    }
    Ok(false)
}

/// Tat ca accounts da luu (cho export/list).
pub fn list_accounts(app_data: Option<PathBuf>) -> Result<Vec<SavedConfig>, String> {
    let map = load_accounts_raw(&app_data)?;
    if map.is_empty() {
        let single = load_config(app_data)?;
        if !single.user_id.is_empty() || !single.jwt.is_empty() {
            return Ok(vec![single]);
        }
    }
    Ok(map.into_values().collect())
}

// ── AutoClaw accounts (OAuth Quoc te) ──

/// File rieng cho AutoClaw: khong tron vao accounts.json cua ZCode (dang khac
/// nhau: token/refresh/device cua AutoClaw, khong co JWT/deviceMid ZCode).
/// File: autoclaw-accounts.json  { "user_id": AutoClawAccount, ... }
/// user_id o day la id THO (khong tien to "autoclaw:") — frontend them tien to
/// de khoi dung id ZCode khi hien chung 1 bang.
const AUTOCLAW_ACCOUNTS_FILE: &str = "autoclaw-accounts.json";

/// 1 tai khoan AutoClaw da dang nhap OAuth (token lay tu oauth-login, co
/// refresh_token de sau nay lam refresh; chua co refresh — het han thi
/// dang nhap lai).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AutoClawAccount {
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub user_id: String,
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub user_name: String,
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub email: String,
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub token: String,
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub refresh_token: String,
    #[serde(default, deserialize_with = "empty_string_if_null")]
    pub device_id: String,
    /// Han token (unix ms, tu JWT exp) — de hien thi, chua dung de refresh.
    #[serde(default)]
    pub expires_at: Option<f64>,
}

fn autoclaw_accounts_path(app_data: &Option<PathBuf>) -> Result<PathBuf, String> {
    let dir = app_data.clone().unwrap_or_else(|| app_data_dir().unwrap_or_else(|_| PathBuf::from(".")));
    Ok(dir.join(AUTOCLAW_ACCOUNTS_FILE))
}

fn load_autoclaw_accounts_raw(
    app_data: &Option<PathBuf>,
) -> Result<std::collections::HashMap<String, AutoClawAccount>, String> {
    let path = autoclaw_accounts_path(app_data)?;
    match read_json(&path)? {
        Some(serde_json::Value::Object(map)) => {
            let mut out = std::collections::HashMap::new();
            for (k, v) in map {
                if let Ok(cfg) = serde_json::from_value::<AutoClawAccount>(v) {
                    out.insert(k, cfg);
                }
            }
            Ok(out)
        }
        Some(_) => Ok(std::collections::HashMap::new()),
        None => Ok(std::collections::HashMap::new()),
    }
}

fn save_autoclaw_accounts_raw(
    app_data: &Option<PathBuf>,
    map: &std::collections::HashMap<String, AutoClawAccount>,
) -> Result<(), String> {
    let path = autoclaw_accounts_path(app_data)?;
    let v = serde_json::to_value(map).map_err(|e| e.to_string())?;
    write_json(&path, &v)
}

/// Tat ca tai khoan AutoClaw da luu.
pub fn list_autoclaw_accounts(app_data: Option<PathBuf>) -> Result<Vec<AutoClawAccount>, String> {
    Ok(load_autoclaw_accounts_raw(&app_data)?.into_values().collect())
}

/// Luu/them 1 tai khoan AutoClaw (key = user_id tho).
pub fn save_autoclaw_account(app_data: Option<PathBuf>, account: &AutoClawAccount) -> Result<(), String> {
    let key = account.user_id.trim();
    if key.is_empty() {
        return Err("AutoClaw account thieu user_id".to_string());
    }
    let mut map = load_autoclaw_accounts_raw(&app_data)?;
    map.insert(key.to_string(), account.clone());
    save_autoclaw_accounts_raw(&app_data, &map)
}

/// Xoa 1 tai khoan AutoClaw (chi xoa file app nay, khong anh huong login desktop).
pub fn delete_autoclaw_account(app_data: Option<PathBuf>, user_id: &str) -> Result<bool, String> {
    let key = user_id.trim();
    if key.is_empty() {
        return Ok(false);
    }
    let mut map = load_autoclaw_accounts_raw(&app_data)?;
    if map.remove(key).is_some() {
        save_autoclaw_accounts_raw(&app_data, &map)?;
        return Ok(true);
    }
    Ok(false)
}

