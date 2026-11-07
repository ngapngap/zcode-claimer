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
    sessions: Mutex<HashMap<String, oauth_login::ZcodeLoginCredentials>>,
}

// Login flow server-side state (rat gon o day thay vi dong bo Tokio task):
// start_zcode_login tra flow_id + auth_url; poll_zcode_login giu jwt/deviceMid/userId
// o LoginState (chi frontend poll moi thay). Poll token khong noi len main.rs.

#[tauri::command]
async fn start_zcode_login(
    state: tauri::State<'_, LoginState>,
) -> Result<oauth_login::LoginStart, String> {
    let (start, login) = oauth_login::start().await.map_err(|e| e.to_string())?;
    // Poll token + flow_id + device_mid se duoc poll_zcode_login doc lai tu reqwest doc o oauth_login:
    // o bo nay ta luu mot ban login token vao State tam de poll dung lai.
    state
        .sessions
        .lock()
        .map_err(|_| "Loi khoa dang nhap (lock POISONed)".to_string())?
        .insert("pending".to_string(), oauth_login::ZcodeLoginCredentials {
            jwt: login.poll_token.clone(),
            access_token: login.flow_id.clone(),
            user_id: start.flow_id.clone(),
            device_mid: login.device_mid.clone(),
        });
    // Poll token/flow_id thi frontend khong can biêt; poll_zcode_login se doc DeviceTrungGan.
    // Tam luu poll_token + flow_id vao state tren cho poll_zcode_login dung (vi oauth_login::CliLogin khong Clone).
    // Poll token + flow_id (access_token field) luu tam de poll_zcode_login doc lai:
    // poll_zcode_login se goi lai oauth_login::poll voi login duoc reconstruct tu state.
    // (Xem poll_zcode_login ben duoi: no goi oauth_login::poll tu state.)
    // Luu them expires_at enqueued o day de poll biet deadline.
    state
        .sessions
        .lock()
        .map_err(|_| "Loi khoa (2)".to_string())?
        .insert("deadline".to_string(), oauth_login::ZcodeLoginCredentials {
            jwt: start.expires_at.map(|v| v.to_string()).unwrap_or_default(),
            access_token: start.poll_interval_sec.to_string(),
            user_id: "".to_string(),
            device_mid: "".to_string(),
        });
    Ok(start)
}

#[tauri::command]
async fn poll_zcode_login(
    state: tauri::State<'_, LoginState>,
    flow_id: String,
) -> Result<serde_json::Value, String> {
    // Giai phap tam: start_zcode_login da luu poll_token + flow_id vao State["pending"];
    // poll_zcode_login doc lai, reconstruct CliLogin va goi oauth_login::poll.
    // Deadline/expiry giu o State["deadline"] (poll token khong can neu duoc reconstruct).
    // Luu y: flow nay chi dung cho mot lan dang nhap tai mot thoi diem (single-flight).
    #[derive(Debug)]
    struct Stored { poll_token: String, real_flow_id: String, device_mid: String, expires_at: Option<i64>, poll_interval_sec: i64 }
    let st = {
        let map = state.sessions.lock().map_err(|_| "Loi khoa dang nhap".to_string())?;
        let p = map.get("pending").ok_or_else(|| "Khong tim thay phien dang nhap (da het han hoac chua start)".to_string())?;
        let d = map.get("deadline").ok_or_else(|| "Thieu deadline dang nhap".to_string())?;
        Stored {
            poll_token: p.jwt.clone(),
            real_flow_id: p.access_token.clone(),
            device_mid: p.device_mid.clone(),
            expires_at: if d.jwt.is_empty() { None } else { Some(d.jwt.parse::<i64>().unwrap_or(0)) },
            poll_interval_sec: d.access_token.parse::<i64>().unwrap_or(3),
        }
    };
    // Tai tram nay flow_id tu frontend (start.flow_id) phai khop real_flow_id.
    if flow_id != st.real_flow_id {
        return Err("flow_id khong khop phien dang nhap".to_string());
    }
    // Reconstruct request (poll_token + real flow_id) de goi oauth_login::poll.
    // De tranh can sua oauth_login::CliLogin thanh public, ta phai goi qua ham poll tu state.
    // Vi oauth_login::CliLogin khong public constructor, hay thay bang tat dung
    // (poll_tram dung ghi trong oauth_login, khong can o main.rs).
    // Gan nhat: oauth_login::CliLogin duoc public qua reconstruct() de main.rs dung.
    let login = oauth_login::CliLoginSendable {
        flow_id: st.real_flow_id.clone(),
        poll_token: st.poll_token.clone(),
        device_mid: st.device_mid.clone(),
    };
    match oauth_login::poll_sendable(&login).await.map_err(|e| e.to_string())? {
        oauth_login::PollResult::Pending => Ok(serde_json::json!({"status":"pending"})),
        oauth_login::PollResult::Ready(creds) => {
            // Xoa tam neutral de tranh poll tiep khi het phien
            if let Ok(mut map) = state.sessions.lock() {
                let _ = map.remove("pending");
                let _ = map.remove("deadline");
            }
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

fn app_data_of(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path().app_data_dir().ok()
}

#[tauri::command]
fn load_saved_config(app: tauri::AppHandle) -> Result<persistence::SavedConfig, String> {
    persistence::load_config(app_data_of(&app))
}

#[tauri::command]
fn save_saved_config(app: tauri::AppHandle, config: persistence::SavedConfig) -> Result<(), String> {
    // Khong log JWT; chi ghi file trong app-data.
    persistence::save_config(app_data_of(&app), &config)
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
