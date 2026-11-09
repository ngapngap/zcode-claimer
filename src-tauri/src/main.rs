#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod activation;
mod autoclaw_balance;
mod autoclaw_checkin;
mod autoclaw_credentials;
mod autoclaw_crypto;
mod autoclaw_oauth;
mod autoclaw_refresh;
mod balance;
mod captcha_config;
mod claim;
mod oauth_login;
mod persistence;

use std::sync::Mutex;
use std::collections::HashMap;

#[derive(Default)]
struct LoginState {
    sessions: Mutex<HashMap<String, oauth_login::CliLogin>>,
}

// Login flow state: key = flow_id, value = CliLogin (poll_token + device_mid do caller cung cap).
// DeviceMid on dinh theo account lay tu persistence o request start; khong gen UUID moi moi lan login.
const DEVICE_MID_FILE: &str = "device-mid.txt";

fn app_data_of(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path().app_data_dir().ok()
}

fn device_mid_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    app_data_of(app)
        .map(|d| d.join(DEVICE_MID_FILE))
        .unwrap_or_else(|| std::path::PathBuf::from(".").join(DEVICE_MID_FILE))
}

fn ensure_device_mid(app: &tauri::AppHandle) -> Result<String, String> {
    let path = device_mid_path(app);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let mid = text.trim().to_string();
        if !mid.is_empty() {
            return Ok(mid);
        }
    }
    let mid = uuid::Uuid::new_v4().to_string();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, &mid);
    Ok(mid)
}

#[tauri::command]
async fn start_zcode_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, LoginState>,
) -> Result<oauth_login::LoginStart, String> {
    let device_mid = ensure_device_mid(&app)?;
    let (start, login) = oauth_login::start(&device_mid).await?;

    state
        .sessions
        .lock()
        .map_err(|_| "Loi khoa dang nhap".to_string())?
        .insert(start.flow_id.clone(), login);
    Ok(start)
}

#[tauri::command]
async fn poll_zcode_login(
    state: tauri::State<'_, LoginState>,
    flow_id: String,
) -> Result<serde_json::Value, String> {
    let login = {
        let map = state.sessions.lock().map_err(|_| "Loi khoa dang nhap".to_string())?;
        map.get(&flow_id).cloned().ok_or_else(|| {
            "Khong tim thay phien dang nhap (da het han hoac chua start)".to_string()
        })?
    };
    match oauth_login::poll(&login).await.map_err(|e| e.to_string())? {
        oauth_login::PollResult::Pending => Ok(serde_json::json!({"status":"pending"})),
        oauth_login::PollResult::Ready(creds) => {
            if let Ok(mut map) = state.sessions.lock() {
                let _ = map.remove(&flow_id);
            }
            // device_mid on dinh theo account (lay tu persistence khi start),
            // khong de runner gen UUID moi moi lan login (docs/protocol.md).
            Ok(serde_json::json!({"status":"ready","credentials":creds}))
        }
    }
}

#[tauri::command]
async fn preview_plans(jwt: String, device_mid: String, user_id: String) -> Result<serde_json::Value, String> {
    let outcome = claim::preview(&jwt, &device_mid).await.map_err(|e| e.to_string())?;
    let (plans, deployed) = match outcome {
        PreviewOutcome::NotDeployed => (Vec::new(), false),
        PreviewOutcome::Plans(v) if !v.is_empty() => (v, true),
        // Empty preview may simply mean the account was not registered as
        // active today (eligibility gate in activation.rs). Report
        // app_launch / app_daily_active, then preview once more, like
        // Agent2API zcode_claim::preview. Failure to report only logs in
        // the response fields — it does not fail this preview.
        PreviewOutcome::Plans(_) => {
            let (activated, activation_error) = match activation::report(&user_id, &device_mid).await {
                Ok(()) => (Some(true), None),
                Err(e) => (Some(false), Some(e)),
            };
            let (plans, deployed) = match claim::preview(&jwt, &device_mid).await {
                Ok(PreviewOutcome::NotDeployed) => (Vec::new(), false),
                Ok(PreviewOutcome::Plans(p)) => (p, true),
                Err(_) => (Vec::new(), true),
            };
            return Ok(serde_json::json!({
                "plans": plans,
                "deployed": deployed,
                "activated": activated,
                "activationError": activation_error,
            }));
        }
    };
    Ok(serde_json::json!({
        "plans": plans,
        "deployed": deployed,
        "activated": null,
        "activationError": null,
    }))
}

#[tauri::command]
async fn claim_plan(
    jwt: String,
    device_mid: String,
    user_id: String,
    plan_id: String,
    captcha_verify_param: String,
    captcha_region: String,
) -> Result<ClaimOutcome, String> {
    // Giai quyen m02612: bao cao kich hoat truoc khi claim (cung nhu
    // Agent2API zcode_claim::claim_plan). Loi bao cao khong chan claim.
    match activation::report(&user_id, &device_mid).await {
        Ok(()) => {}
        Err(_) => {}
    }
    claim::claim(
        &jwt,
        &device_mid,
        &plan_id,
        &captcha_verify_param,
        &captcha_region,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_captcha_config() -> Result<Option<CaptchaConfig>, String> {
    captcha_config::fetch().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn report_activation(user_id: String, device_mid: String) -> Result<(), String> {
    activation::report(&user_id, &device_mid).await.map_err(|e| e.to_string())
}

pub type BalanceResult = balance::BalanceResult;

/// Persist buyer token mới sau refresh vào autoclaw-accounts.json của app
/// (KHÔNG ghi đè auth.json của desktop — sẽ phá login AutoClaw desktop +
/// xung đột luân phiên token).
/// - `user_id` (id THÔ không prefix, do frontend truyền) phải có; không có
///   thì chỉ trả `_refreshed` cho frontend tự lưu (đường local/env).
/// - Tài khoản OAuth (`kind = Manual` tương đương) mới persist
///   (Agent2API `persistRefresh=true`); credential đọc từ login desktop
///   (file) khôngpersist (Agent2API `persistRefresh=false` — đọc là đủ,
///   desktop tự giữ token của nó).
/// - Token đổi thì ghi file + kèm `_refreshed {token, refresh_token, expires_at}`
///   để frontend đồng bộ vào phiên hiện tại.
fn persist_refreshed(
    app: &tauri::AppHandle,
    credentials: &autoclaw_credentials::AutoClawCredentials,
    orig_token: &str,
    user_id: Option<&str>,
    result: &mut serde_json::Value,
) -> Result<(), String> {
    // Ghi chú (m02688): chỉ persist khi token thực sự đổi (khác orig_token).
    // Tài khoản local/env không có dòng trong autoclaw-accounts.json -> không
    // ghi đè, chỉ kèm _refreshed cho frontend tự lưu vào phiên hiện tại.
    if credentials.token == orig_token {
        return Ok(());
    }
    let refreshed = serde_json::json!({
        "token": credentials.token,
        "refresh_token": credentials.refresh_token,
        "expires_at": credentials.expires_at,
    });
    let Some(user_id) = user_id.map(str::trim).filter(|id| !id.is_empty()) else {
        attach(result, &refreshed);
        return Ok(());
    };
    // Chỉ persist khi file autoclaw-accounts.json đã có dòng này (OAuth).
    // Không có dòng (đọc từ desktop/env) -> không ghi đè, chỉ kèm _refreshed.
    let existing = persistence::list_autoclaw_accounts(app_data_of(app));
    let existing_accounts = match &existing {
        Ok(list) => list.clone(),
        Err(_) => Vec::new(),
    };
    let has_account = existing_accounts
        .iter()
        .any(|a| a.user_id.trim() == user_id);
    if has_account {
        let found = existing_accounts
            .iter()
            .find(|a| a.user_id.trim() == user_id);
        let user_name = found.map(|a| a.user_name.clone()).unwrap_or_default();
        let email = found
            .map(|a| a.email.clone())
            .unwrap_or_else(|| credentials.email.clone());
        let record = persistence::AutoClawAccount {
            user_id: user_id.to_string(),
            user_name,
            email,
            token: credentials.token.clone(),
            refresh_token: credentials.refresh_token.clone(),
            device_id: credentials.device_id.clone(),
            expires_at: credentials.expires_at,
        };
        let _ = persistence::save_autoclaw_account(app_data_of(app), &record);
    }
    attach(result, &refreshed);
    Ok(())
}

fn attach(result: &mut serde_json::Value, refreshed: &serde_json::Value) {
    if let Some(obj) = result.as_object_mut() {
        obj.insert("_refreshed".to_string(), refreshed.clone());
    } else {
        *result = serde_json::json!({ "value": result.clone(), "_refreshed": refreshed });
    }
}

#[tauri::command]
async fn query_balance(jwt: String, device_mid: String) -> Result<BalanceResult, String> {
    balance::query_balance(&jwt, &device_mid).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn query_autoclaw_balance(
    app: tauri::AppHandle,
    token: String,
    refresh_token: Option<String>,
    device_id: Option<String>,
    region: Option<String>,
    user_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let region = match region.as_deref() {
        Some("intl") => autoclaw_credentials::Region::Intl,
        _ => autoclaw_credentials::Region::Cn,
    };
    let mut credentials = if token.trim().is_empty() {
        autoclaw_credentials::local_credentials(region)
            .or_else(|_| autoclaw_credentials::env_credentials(region).ok_or_else(|| "Khong thay dang nhap AutoClaw".to_string()))
    } else {
        autoclaw_credentials::credentials_from_token(region, &token, refresh_token.as_deref().unwrap_or(""), device_id.as_deref().unwrap_or(""))
    }?;
    // m02688: token AutoClaw hết hạn sau ~1 ngày. Tự refresh access token
    // từ refresh token trước khi gọi balance (ensure_fresh), và nếu balance
    // vẫn báo hết hạn/auth (401/code 410000/400000) thì refresh bắt buộc
    // rồi gọi lại 1 lần (refresh_access_token của Agent2API adapter.rs).
    // Chỉ refresh khi có refresh_token (OAuth); tài khoản đọc từ login
    // desktop không có refresh_token thì giữ nguyên hành vi cũ.
    let orig_token = credentials.token.clone();
    match autoclaw_refresh::ensure_fresh(&credentials).await {
        Ok(next) => credentials = next,
        Err(e) => return Err(e),
    }
    let mut result = autoclaw_balance::query_balance(&credentials).await;
    if result.is_err() && credentials.token != orig_token && autoclaw_refresh::can_refresh(&credentials) {
        // Token cũ vừa bị thay bằng ensure_fresh nhưng vẫn lỗi -> force refresh từ refresh trong credential mới (nếu có)
        if let Ok(next) = autoclaw_refresh::refresh(&credentials, true).await {
            credentials = next;
            result = autoclaw_balance::query_balance(&credentials).await;
        }
    } else if result.is_err() && autoclaw_refresh::can_refresh(&credentials) {
        // Lỗi do token gốc hỏng (vd vừa hết hạn trong lúc chạy): ép refresh 1 lần rồi gọi lại
        if let Ok(next) = autoclaw_refresh::refresh(&credentials, true).await {
            credentials = next;
            result = autoclaw_balance::query_balance(&credentials).await;
        }
    }
    let balance = result?;
    let mut value = serde_json::to_value(&balance).map_err(|e| e.to_string())?;
    persist_refreshed(&app, &credentials, &orig_token, user_id.as_deref(), &mut value)?;
    Ok(value)
}

/// Diem danh hang ngay AutoClaw (task daily_signin + kiem tra qua hoat dong).
/// `today` la ngay Bac Kinh ("YYYY-MM-DD") do frontend truyen — so sanh voi
/// `grant_date` trong so diem (xem autoclaw_checkin.rs).
#[tauri::command]
async fn autoclaw_daily_checkin(
    app: tauri::AppHandle,
    token: String,
    refresh_token: Option<String>,
    device_id: Option<String>,
    region: Option<String>,
    user_id: Option<String>,
    today: String,
) -> Result<serde_json::Value, String> {
    let region = match region.as_deref() {
        Some("intl") => autoclaw_credentials::Region::Intl,
        _ => autoclaw_credentials::Region::Cn,
    };
    let mut credentials = if token.trim().is_empty() {
        autoclaw_credentials::local_credentials(region)
            .or_else(|_| autoclaw_credentials::env_credentials(region).ok_or_else(|| "Khong thay dang nhap AutoClaw".to_string()))
    } else {
        autoclaw_credentials::credentials_from_token(region, &token, refresh_token.as_deref().unwrap_or(""), device_id.as_deref().unwrap_or(""))
    }?;
    let orig_token = credentials.token.clone();
    match autoclaw_refresh::ensure_fresh(&credentials).await {
        Ok(next) => credentials = next,
        Err(e) => return Err(e),
    }
    let mut result = autoclaw_checkin::daily_checkin(&credentials, &today).await;
    if result.is_err() && autoclaw_refresh::can_refresh(&credentials) {
        if let Ok(next) = autoclaw_refresh::refresh(&credentials, true).await {
            credentials = next;
            result = autoclaw_checkin::daily_checkin(&credentials, &today).await;
        }
    }
    let mut result = result?;
    persist_refreshed(&app, &credentials, &orig_token, user_id.as_deref(), &mut result)?;
    Ok(result)
}

// ── AutoClaw Quoc te OAuth login ──

#[tauri::command]
async fn autoclaw_oauth_captcha_config() -> Result<serde_json::Value, String> {
    autoclaw_oauth::captcha_config().await
}

#[tauri::command]
async fn start_autoclaw_oauth(
    vendor: Option<String>,
    captcha_verify_param: String,
) -> Result<serde_json::Value, String> {
    autoclaw_oauth::start_oauth(vendor.as_deref().unwrap_or("zai"), &captcha_verify_param).await
}

#[tauri::command]
async fn poll_autoclaw_oauth(flow_id: String) -> Result<serde_json::Value, String> {
    autoclaw_oauth::poll_oauth(&flow_id)
}

#[tauri::command]
async fn cancel_autoclaw_oauth(flow_id: String) -> Result<(), String> {
    autoclaw_oauth::cancel_oauth(&flow_id);
    Ok(())
}

#[tauri::command]
fn list_autoclaw_accounts(app: tauri::AppHandle) -> Result<Vec<persistence::AutoClawAccount>, String> {
    persistence::list_autoclaw_accounts(app_data_of(&app))
}

#[tauri::command]
fn save_autoclaw_account(
    app: tauri::AppHandle,
    account: persistence::AutoClawAccount,
) -> Result<(), String> {
    persistence::save_autoclaw_account(app_data_of(&app), &account)
}

#[tauri::command]
fn delete_autoclaw_account(app: tauri::AppHandle, user_id: String) -> Result<bool, String> {
    persistence::delete_autoclaw_account(app_data_of(&app), &user_id)
}

#[tauri::command]
fn generate_device_mid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[tauri::command]
fn export_router_json(
    jwt: String,
    user_id: String,
    username: String,
    email: String,
    zai_business_token: String,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let json = serde_json::json!({
        "id": id,
        "provider": "glm",
        "authType": "oauth",
        "name": "ZCode Daily Claim",
        "email": email,
        "priority": 100,
        "isActive": true,
        "data": {
            "providerSpecificData": {
                "authMethod": "zcode-jwt",
                "username": username,
                "userId": user_id,
                "zcodeJwtToken": jwt,
                "zaiBusinessToken": zai_business_token
            },
            "testStatus": "active"
        }
    });
    serde_json::to_string_pretty(&json).unwrap_or_default()
}

// Entry router cho tài khoản AutoClaw: mirror đúng shape entry ZCode
// (provider glm, authType oauth) nhưng dùng access token AutoClaw;
// email/user_name để nhận biết (như entry ZCode dùng email).
// Refresh token đi kèm để router tự làm mới access token khi hết hạn —
// thiếu nó thì entry AutoClaw chết ngay khi access token hết hạn.
#[tauri::command]
fn export_router_json_autoclaw(
    token: String,
    refresh_token: String,
    user_id: String,
    username: String,
    email: String,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let json = serde_json::json!({
        "id": id,
        "provider": "glm",
        "authType": "oauth",
        "name": "AutoClaw",
        "email": email,
        "priority": 100,
        "isActive": true,
        "data": {
            "providerSpecificData": {
                "authMethod": "autoclaw-oauth",
                "username": username,
                "userId": user_id,
                "autoclawAccessToken": token,
                "autoclawRefreshToken": refresh_token
            },
            "testStatus": "active"
        }
    });
    serde_json::to_string_pretty(&json).unwrap_or_default()
}

// ── Persistence commands (config + claim history trong app-data) ──

#[tauri::command]
fn load_saved_config(app: tauri::AppHandle) -> Result<persistence::SavedConfig, String> {
    persistence::load_config(app_data_of(&app))
}

#[tauri::command]
fn save_saved_config(app: tauri::AppHandle, config: persistence::SavedConfig) -> Result<(), String> {
    // Khong log JWT; chi ghi file trong app-data. Multi-account: key=user_id.
    persistence::save_account(app_data_of(&app), &config)
}

#[tauri::command]
fn list_accounts(app: tauri::AppHandle) -> Result<Vec<persistence::SavedConfig>, String> {
    persistence::list_accounts(app_data_of(&app))
}

#[tauri::command]
fn delete_account(app: tauri::AppHandle, user_id: String) -> Result<bool, String> {
    persistence::delete_account(app_data_of(&app), &user_id)
}

#[tauri::command]
fn save_text_file(path: String, contents: String) -> Result<(), String> {
    // Ghi file ra duong dan user chon trong hop thoai Save (khong qua plugin-fs
    // de khoi can cau hinh scope; lenh Rust tu viet khong bi gioi han scope).
    std::fs::write(&path, contents).map_err(|e| format!("khong ghi duoc file {}: {}", path, e))
}

#[tauri::command]
fn list_claim_history(app: tauri::AppHandle) -> Result<Vec<persistence::ClaimHistoryEntry>, String> {
    persistence::load_history(app_data_of(&app))
}

#[tauri::command]
fn record_claim_history(
    app: tauri::AppHandle,
    plan_id: String,
    date: String,
    outcome: String,
    code: i64,
    message: String,
    failure_ends_at: Option<i64>,
) -> Result<Vec<persistence::ClaimHistoryEntry>, String> {
    let recorded_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    persistence::append_history(
        app_data_of(&app),
        persistence::ClaimHistoryEntry {
            plan_id,
            date,
            outcome,
            code,
            message,
            failure_ends_at,
            recorded_at,
        },
    )
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(LoginState::default())
        .invoke_handler(tauri::generate_handler![
            preview_plans,
            claim_plan,
            get_captcha_config,
            report_activation,
            generate_device_mid,
            export_router_json,
            export_router_json_autoclaw,
            query_balance,
            query_autoclaw_balance,
            autoclaw_daily_checkin,
            autoclaw_oauth_captcha_config,
            start_autoclaw_oauth,
            poll_autoclaw_oauth,
            cancel_autoclaw_oauth,
            list_autoclaw_accounts,
            save_autoclaw_account,
            delete_autoclaw_account,
            load_saved_config,
            save_saved_config,
            list_accounts,
            delete_account,
            save_text_file,
            list_claim_history,
            record_claim_history,
            start_zcode_login,
            poll_zcode_login
        ])
        .run(tauri::generate_context!())
        .expect("error while running zcode-claimer");
}

// ── Type aliases ──
pub type PreviewOutcome = claim::PreviewOutcome;
pub type ClaimOutcome = claim::ClaimOutcome;
pub type CaptchaConfig = captcha_config::CaptchaConfig;
