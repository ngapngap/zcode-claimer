#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod claim;
mod captcha_config;
mod activation;

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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            preview_plans,
            claim_plan,
            get_captcha_config,
            report_activation,
            generate_device_mid,
            export_router_json
        ])
        .run(tauri::generate_context!())
        .expect("error while running zcode-claimer");
}

// ── Type aliases ──
pub type PreviewOutcome = claim::PreviewOutcome;
pub type ClaimOutcome = claim::ClaimOutcome;
pub type CaptchaConfig = captcha_config::CaptchaConfig;
