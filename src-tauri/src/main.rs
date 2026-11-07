#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod activation;
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
async fn preview_plans(jwt: String, device_mid: String) -> Result<PreviewOutcome, String> {
    claim::preview(&jwt, &device_mid).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn claim_plan(
    jwt: String,
    device_mid: String,
    plan_id: String,
    captcha_verify_param: String,
    captcha_region: String,
) -> Result<ClaimOutcome, String> {
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
async fn report_activation(jwt: String, device_mid: String) -> Result<(), String> {
    activation::report(&jwt, &device_mid).await.map_err(|e| e.to_string())
}

pub type BalanceResult = balance::BalanceResult;

#[tauri::command]
async fn query_balance(jwt: String, device_mid: String) -> Result<BalanceResult, String> {
    balance::query_balance(&jwt, &device_mid).await.map_err(|e| e.to_string())
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
        .manage(LoginState::default())
        .invoke_handler(tauri::generate_handler![
            preview_plans,
            claim_plan,
            get_captcha_config,
            report_activation,
            generate_device_mid,
            export_router_json,
            query_balance,
            load_saved_config,
            save_saved_config,
            list_accounts,
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
