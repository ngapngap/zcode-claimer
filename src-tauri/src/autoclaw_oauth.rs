//! AutoClaw **Quoc te** OAuth login (giao thuc OAuth cua client AutoClaw:
//! oauth + callback tren port dang ky + web login, rut gon cho app nay).
//!
//! Luong (chi Quoc te — Trung Quoc khong co OAuth, da bo theo yeu cau):
//! ```text
//! 1) POST {intl}/userapi/overseasv1/oauth-captcha-config  {}
//!      -> data {enabled, region, prefix, scene_id, captcha_supplier}
//!      (enabled=false = khong can OAuth; Quoc te thi enabled=true)
//! 2) Frontend chay slider captcha Aliyun (solver chung ui/aliyun-captcha.js)
//!      -> verifyParam (opaque, khong tu tao duoc)
//! 3) POST {intl}/userapi/overseasv1/{zai|google}-oauth-url
//!      {source_id:"autoclaw", device_id, navigate_uri, ali_captcha_verify_param}
//!      -> data.oauth_url (trang dang nhap chinh thuc)
//! 4) Trinh duyet dang nhap xong -> 302 ve
//!      http://localhost:<port>/auth/callback-{zai|google}?code=..&state=..
//!      (port PHAI la 1 trong 4 port z.ai da dang ky cho client chinh thuc:
//!       18432 / 19654 / 19723 / 53699; sai port/host/path deu bi tu choi
//!       "Redirect URI not registered for this client")
//! 5) POST {intl}/userapi/overseasv1/{zai|google}-oauth-login
//!      {source_id, device_id, code, state, navigate_uri}
//!      -> data {access_token, refresh_token, user_id, user_name, first_login}
//! ```
//!
//! Buoc 3 BAT BUOC co captcha: khong co verifyParam -> 631002 (ma gay hieu lam
//! "phien ban da ngung"); gia tri gia -> 630014. Buoc 5 khong can captcha.
//! Cac request deu ky bang signed_auth_headers (an danh, khong token) nhu
//! balance/refresh — cung cach ky nhu cac request userapi khac.
//!
//! Frontend poll `poll_oauth(flow_id)` moi 1 giay (giong ZCode CLI poll):
//! pending -> ready {credentials} | failed {message}. Het 5 phut frontend
//! tu cancel. Listener chi bind loopback (127.0.0.1, them ::1 neu duoc) va
//! chi nhan 2 duong callback da biet, cac duong khac 404.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

use crate::autoclaw_balance::signed_auth_headers;
use crate::autoclaw_credentials::Region;
use crate::autoclaw_crypto;

/// z.ai dang ky cho client AutoClaw chinh thuc: chi 4 port nay moi duoc lam
/// redirect_uri (doc tu app.asar cua client: ALL_PORTS).
const REGISTERED_CALLBACK_PORTS: [u16; 4] = [18432, 19654, 19723, 53699];

/// Tien to duong callback (giong client: /auth/callback-zai | -google).
const CALLBACK_PATH_PREFIX: &str = "/auth/callback-";

/// Timeout request OAuth (giong client: 15 giay).
const REQUEST_TIMEOUT_MS: u64 = 15_000;

/// Frontend poll moi 1 giay (callback la local, do tre thap).
const POLL_INTERVAL_MS: u64 = 1_000;

/// Tien to flow id (de nhan dien trong log).
const FLOW_PREFIX: &str = "autoclaw-oauth-";

/// Base userapi Quoc te (Region::Intl). Chi Quoc te — khong co Cn o day.
fn intl_base_url() -> String {
    Region::Intl.userapi_base_url().to_string()
}

fn describe_oauth_error(code: i64, upstream_msg: &str) -> String {
    match code {
        631_002 => "Lan dang nhap nay thieu xac thuc chong bot (captcha chua qua hoac da het han), bam Dang nhap lai".to_string(),
        630_014 => "Xac thuc chong bot khong qua, bam Dang nhap lai".to_string(),
        631_001 => "Ma xac thuc het han hoac khong hop le, bam Dang nhap lai".to_string(),
        400_001 => "Tham so yeu cau sai (loi noi bo luong dang nhap, thu lai)".to_string(),
        400_002 => "Xac thuc chu ky that bai (dong ho may co lech), chinh lai gio he thong roi thu lai".to_string(),
        _ => {
            if upstream_msg.trim().is_empty() {
                format!("Dang nhap that bai (upstream code={code})")
            } else {
                format!("Dang nhap that bai: {}", upstream_msg.trim())
            }
        }
    }
}

/// POST co ky (an danh) len userapi Quoc te, tra ve payload JSON.
async fn post_signed(path: &str, body: &Value, what: &str) -> Result<Value, String> {
    let url = format!("{}{path}", intl_base_url());
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(REQUEST_TIMEOUT_MS))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.post(&url).json(body);
    for (k, v) in signed_auth_headers("") {
        req = req.header(k, v);
    }
    let response = req
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!("{what} timeout")
            } else {
                format!("{what} that bai: {e}")
            }
        })?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        return Err(format!("{what} tra ve HTTP {status}"));
    }
    response
        .json::<Value>()
        .await
        .map_err(|_| format!("{what}: phan hoi khong phai JSON"))
}

/// Lay cau hinh captcha (dong thoi la cong tac "co can OAuth khong").
///
/// Tra ve dang chuan hoa {enabled, region, prefix, sceneId, supplier}
/// de frontend dua thang vao solver.
pub async fn captcha_config() -> Result<Value, String> {
    let payload = post_signed(
        "/userapi/overseasv1/oauth-captcha-config",
        &json!({}),
        "Lay cau hinh xac thuc",
    )
    .await?;
    let code = payload.get("code").and_then(Value::as_i64).unwrap_or(0);
    if code != 0 {
        let upstream_msg = payload.get("msg").and_then(Value::as_str).unwrap_or("");
        return Err(describe_oauth_error(code, upstream_msg));
    }
    let data = payload.get("data").cloned().unwrap_or(Value::Null);
    let text = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    Ok(json!({
        "enabled": data.get("enabled").and_then(Value::as_bool).unwrap_or(false),
        "region": text("region"),
        "prefix": text("prefix"),
        "sceneId": text("scene_id"),
        "supplier": text("captcha_supplier"),
    }))
}

/// 32 byte ngau nhien -> 64 hex (device id cho OAuth;
/// new_device_id — hai buoc oauth-url / oauth-login phai dung chung 1 gia tri).
fn new_oauth_device_id() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        let fallback = format!(
            "{:x}{:x}{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|v| v.as_nanos())
                .unwrap_or(0),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|v| v.subsec_nanos())
                .unwrap_or(0)
        );
        return format!("{fallback:0>64}").chars().take(64).collect();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex_16() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|v| v.as_nanos())
            .unwrap_or(0);
        return format!("{nanos:032x}");
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Ghep navigate_uri (dong thoi la callback URL trinh duyet se 302 ve).
fn navigate_uri(callback_base: &str, vendor: &str) -> String {
    format!(
        "{}{}{}",
        callback_base.trim_end_matches('/'),
        CALLBACK_PATH_PREFIX,
        vendor
    )
}

/// Lay dia chi OAuth (buoc 3). Can verifyParam — khong co thi chan ngay o local
/// (khong co no goi upstream chac chan an 631002 gay hieu lam).
async fn request_oauth_url(
    vendor: &str,
    navigate: &str,
    captcha_verify_param: &str,
    device_id: &str,
) -> Result<String, String> {
    let captcha = captcha_verify_param.trim();
    if captcha.is_empty() {
        return Err("Thieu tham so xac thuc chong bot, hay hoan thanh captcha truoc".to_string());
    }
    let body = json!({
        "source_id": "autoclaw",
        "device_id": device_id,
        "navigate_uri": navigate,
        "ali_captcha_verify_param": captcha,
    });
    let what = format!("Lay dia chi dang nhap {vendor}");
    let payload = post_signed(
        &format!("/userapi/overseasv1/{vendor}-oauth-url"),
        &body,
        &what,
    )
    .await?;
    let code = payload.get("code").and_then(Value::as_i64).unwrap_or(0);
    if code != 0 {
        let upstream_msg = payload.get("msg").and_then(Value::as_str).unwrap_or("");
        return Err(describe_oauth_error(code, upstream_msg));
    }
    let oauth_url = payload
        .get("data")
        .and_then(|d| d.get("oauth_url"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if oauth_url.is_empty() {
        return Err("Upstream khong tra ve dia chi dang nhap, thu lai".to_string());
    }
    Ok(oauth_url)
}

/// Dung code tu callback de doi lay token (buoc 5). Tra ve credential phang
/// {token, refreshToken?, userName?, userId, deviceId, expiresAt?}.
async fn exchange_code(
    vendor: &str,
    code: &str,
    state: &str,
    navigate: &str,
    device_id: &str,
) -> Result<Value, String> {
    let body = json!({
        "source_id": "autoclaw",
        "device_id": device_id,
        "code": code,
        "state": state,
        "navigate_uri": navigate,
    });
    let what = format!("Doi ma xac thuc {vendor} lay token");
    let payload = post_signed(
        &format!("/userapi/overseasv1/{vendor}-oauth-login"),
        &body,
        &what,
    )
    .await?;
    let status = payload.get("code").and_then(Value::as_i64).unwrap_or(0);
    if status != 0 {
        let upstream_msg = payload.get("msg").and_then(Value::as_str).unwrap_or("");
        return Err(describe_oauth_error(status, upstream_msg));
    }
    let data = payload.get("data").cloned().unwrap_or(Value::Null);
    let token = data
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if token.is_empty() {
        return Err("Dang nhap xong nhung upstream khong tra ve access_token".to_string());
    }
    let token = autoclaw_crypto::strip_bearer(&token);
    // user_id: uu tien data.user_id, fallback sang claim trong JWT.
    let mut user_id = data
        .get("user_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let mut expires_at: Option<f64> = None;
    if let Some(claims) = autoclaw_crypto::decode_jwt_claims(&token) {
        if user_id.is_empty() {
            user_id = claims
                .get("user_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
        }
        if let Some(exp) = claims.get("exp").and_then(Value::as_f64) {
            expires_at = Some(exp * 1000.0);
        }
    }
    let refresh_token = data
        .get("refresh_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let user_name = data
        .get("user_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let mut creds = json!({
        "token": token,
        "deviceId": device_id,
        "userId": user_id,
    });
    if let Some(object) = creds.as_object_mut() {
        if !refresh_token.is_empty() {
            object.insert("refreshToken".to_string(), Value::String(refresh_token));
        }
        if !user_name.is_empty() {
            object.insert("userName".to_string(), Value::String(user_name));
        }
        if let Some(exp) = expires_at {
            object.insert("expiresAt".to_string(), json!(exp));
        }
    }
    Ok(creds)
}

// ── Bang flow (trang thai cac luot dang nhap) ──

enum FlowState {
    /// Da co auth_url, doi trinh duyet callback.
    Waiting,
    /// Da nhan code, dang doi token.
    Working,
    /// Doi xong (credential phang).
    Done(Value),
    /// That bai.
    Failed(String),
}

struct FlowEntry {
    vendor: String,
    navigate_uri: String,
    device_id: String,
    state: FlowState,
    shutdown: watch::Sender<bool>,
}

fn flows() -> &'static Mutex<HashMap<String, FlowEntry>> {
    static FLOWS: OnceLock<Mutex<HashMap<String, FlowEntry>>> = OnceLock::new();
    FLOWS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_flows<T>(action: impl FnOnce(&mut HashMap<String, FlowEntry>) -> T) -> T {
    match flows().lock() {
        Ok(mut table) => action(&mut table),
        Err(poisoned) => action(&mut poisoned.into_inner()),
    }
}

fn close_all_flows() {
    let senders: Vec<watch::Sender<bool>> =
        with_flows(|table| table.drain().map(|(_, e)| e.shutdown).collect());
    for sender in senders {
        let _ = sender.send(true);
    }
}

fn set_flow_state(flow_id: &str, state: FlowState) {
    with_flows(|table| {
        if let Some(entry) = table.get_mut(flow_id) {
            entry.state = state;
        }
    });
}

fn close_flow(flow_id: &str) {
    with_flows(|table| {
        if let Some(entry) = table.remove(flow_id) {
            let _ = entry.shutdown.send(true);
        }
    });
}

/// Chi tra port (khong xoa flow): dung khi flow vua ket thuc (Done/Failed)
/// — ket qua phai nam lai trong bang de frontend poll doc duoc, chi co
/// poll_oauth moi duoc xoa entry.
fn close_listener(flow_id: &str) {
    with_flows(|table| {
        if let Some(entry) = table.get(flow_id) {
            let _ = entry.shutdown.send(true);
        }
    });
}

// ── Callback listener (lang nghe tren port dang ky) ──

/// Chiem port dang ky dau tien con trong, tra ve (port, shutdown sender).
/// Tat ca deu bi chiem -> Err (thuong do AutoClaw desktop dang chay).
async fn bind_callback_listener(
    sender: mpsc::UnboundedSender<String>,
) -> Result<(u16, watch::Sender<bool>), String> {
    for port in REGISTERED_CALLBACK_PORTS {
        let Ok(v4) = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await else {
            continue;
        };
        let v6 = TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port))
            .await
            .ok();
        let (shutdown, receiver) = watch::channel(false);
        spawn_server(v4, sender.clone(), receiver.clone());
        if let Some(v6) = v6 {
            spawn_server(v6, sender.clone(), receiver);
        }
        return Ok((port, shutdown));
    }
    Err(format!(
        "Cac cong callback dang nhap AutoClaw ({}) deu bi chiem (thuong do AutoClaw desktop dang chay) — hay thoat AutoClaw desktop roi thu lai",
        REGISTERED_CALLBACK_PORTS
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(" / ")
    ))
}

fn spawn_server(
    listener: TcpListener,
    sender: mpsc::UnboundedSender<String>,
    mut shutdown: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        loop {
            if *shutdown.borrow() {
                return;
            }
            tokio::select! {
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            let sender = sender.clone();
                            tokio::spawn(async move {
                                serve_one(stream, sender).await;
                            });
                        }
                        Err(_) => continue,
                    }
                }
                _ = shutdown.changed() => return,
            }
        }
    });
}

async fn serve_one(mut stream: TcpStream, sender: mpsc::UnboundedSender<String>) {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let read = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read(&mut chunk),
        )
        .await
        {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(read)) => read,
            Ok(Err(_)) => return,
        };
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buffer.len() >= 8192 {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buffer);
    let request_line = head.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let _method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };
    // Chi nhan 2 duong callback da biet — port nay la muon, it tra loi nhat co the.
    if path != "/auth/callback-zai" && path != "/auth/callback-google" {
        respond(&mut stream, 404, "Not Found", "Not Found").await;
        return;
    }
    let vendor = path
        .strip_prefix(CALLBACK_PATH_PREFIX)
        .unwrap_or("")
        .to_string();
    let params: HashMap<String, String> = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let has_code = params
        .get("code")
        .is_some_and(|v| !v.trim().is_empty());
    let has_error = params
        .get("error")
        .is_some_and(|v| !v.trim().is_empty());
    // Khong nhan ra (favicon / preconnect) thi 404, listener tiep tuc doi.
    if !has_code && !has_error {
        respond(&mut stream, 404, "Not Found", "Not Found").await;
        return;
    }
    if sender.send(format!("{vendor}|{query}")).is_err() {
        respond(
            &mut stream,
            410,
            "Login expired",
            "Luot dang nhap nay da ket thuc, hay quay lai app dang nhap lai.",
        )
        .await;
        return;
    }
    if has_error {
        let message = params.get("error").cloned().unwrap_or_default();
        respond(&mut stream, 200, "Login failed", &message).await;
    } else {
        respond(
            &mut stream,
            200,
            "Login successful",
            "Dang nhap thanh cong — ban co the dong cua so nay.",
        )
        .await;
    }
}

async fn respond(stream: &mut TcpStream, status: u16, title: &str, message: &str) {
    let escaped = message.replace('<', "&lt;").replace('>', "&gt;");
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{title}</title>\
         <div style=\"font:16px/1.6 system-ui;padding:40px;text-align:center\">\
         <h2>{title}</h2><p>{escaped}</p></div>"
    );
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        410 => "Gone",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
}

/// Giai ma phan tram (dau + trong query tinh la khoang trang).
fn percent_decode(input: &str) -> String {
    let mut bytes: Vec<u8> = Vec::with_capacity(input.len());
    let raw = input.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'%' if index + 2 < raw.len() => {
                let hex = &input[index + 1..index + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        bytes.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        bytes.push(raw[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                bytes.push(b' ');
                index += 1;
            }
            byte => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Dung query tu callback de doi token (chi 1 cho duy nhat — giong web_login.rs finish).
async fn finish(
    vendor: &str,
    navigate_uri: &str,
    device_id: &str,
    query: &str,
) -> Result<Value, String> {
    let params: HashMap<String, String> = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (percent_decode(k), percent_decode(v)))
        .collect();
    if let Some(error) = params.get("error").filter(|v| !v.trim().is_empty()) {
        return Err(format!("Uy quyen bi tu choi ({error})"));
    }
    let code = params.get("code").map(String::as_str).unwrap_or("").trim();
    if code.is_empty() {
        return Err("Callback khong mang theo ma xac thuc".to_string());
    }
    // state trong query la state do upstream sinh (doi token phai tra lai no),
    // khong phai flow id cua minh.
    let upstream_state = params.get("state").map(String::as_str).unwrap_or("");
    exchange_code(
        vendor,
        code,
        upstream_state.trim(),
        navigate_uri,
        device_id,
    )
    .await
}

// ── API dung cho Tauri command ──

/// Bat dau 1 luot OAuth: chiem port -> lay auth_url -> dang ky flow -> tra ve
/// {flow_id, auth_url, poll_interval_ms}. Frontend mo auth_url bang trinh duyet
/// mac dinh roi poll moi 1 giay.
pub async fn start_oauth(vendor: &str, captcha_verify_param: &str) -> Result<Value, String> {
    let vendor = match vendor.trim() {
        "google" => "google",
        _ => "zai",
    };
    close_all_flows();
    let device_id = new_oauth_device_id();
    let (sender, inbox) = mpsc::unbounded_channel();
    let (port, shutdown) = bind_callback_listener(sender).await?;
    let base = format!("http://localhost:{port}");
    let navigate = navigate_uri(&base, vendor);
    let auth_url = match request_oauth_url(vendor, &navigate, captcha_verify_param, &device_id)
        .await
    {
        Ok(url) => url,
        Err(e) => {
            let _ = shutdown.send(true);
            return Err(e);
        }
    };
    let flow_id = format!("{FLOW_PREFIX}{}", random_hex_16());
    with_flows(|table| {
        table.insert(
            flow_id.clone(),
            FlowEntry {
                vendor: vendor.to_string(),
                navigate_uri: navigate.clone(),
                device_id: device_id.clone(),
                state: FlowState::Waiting,
                shutdown,
            },
        );
    });
    spawn_completion(flow_id.clone(), vendor.to_string(), navigate, device_id, inbox);
    Ok(json!({
        "flow_id": flow_id,
        "auth_url": auth_url,
        "poll_interval_ms": POLL_INTERVAL_MS,
    }))
}

/// Nua sau cua luot dang nhap (chay nen): doi callback -> doi token -> ghi state.
fn spawn_completion(
    flow_id: String,
    vendor: String,
    navigate_uri: String,
    device_id: String,
    mut inbox: mpsc::UnboundedReceiver<String>,
) {
    tokio::spawn(async move {
        let Some(payload) = inbox.recv().await else {
            return; // listener da dong (cancel) truoc khi co callback
        };
        let (callback_vendor, query) = match payload.split_once('|') {
            Some((v, q)) => (Some(v.to_string()), q.to_string()),
            None => (None, payload),
        };
        if let Some(callback_vendor) = callback_vendor {
            if callback_vendor != vendor {
                set_flow_state(
                    &flow_id,
                    FlowState::Failed(format!(
                        "Callback thuoc {callback_vendor}, khong khop luot dang nhap {vendor} nay — dang nhap lai"
                    )),
                );
                close_listener(&flow_id);
                return;
            }
        }
        set_flow_state(&flow_id, FlowState::Working);
        let outcome = finish(&vendor, &navigate_uri, &device_id, &query).await;
        match outcome {
            Ok(creds) => set_flow_state(&flow_id, FlowState::Done(creds)),
            Err(message) => set_flow_state(&flow_id, FlowState::Failed(message)),
        }
        // Chi tra port; entry o lai cho poll_oauth doc ket qua roi moi xoa.
        close_listener(&flow_id);
    });
}

/// Poll 1 lan: pending (co hint) | ready {credentials} | failed {message}.
/// Trang thai ket thuc thi xoa flow + tra port.
pub fn poll_oauth(flow_id: &str) -> Result<Value, String> {
    let result = with_flows(|table| match table.get(flow_id) {
        Some(entry) => Some(match &entry.state {
            FlowState::Waiting => json!({"status":"pending","hint":"Dang cho ban hoan thanh dang nhap trong trinh duyet..."}),
            FlowState::Working => json!({"status":"pending","hint":"Da nhan ma xac thuc, dang doi lay token..."}),
            FlowState::Done(creds) => json!({"status":"ready","credentials":creds}),
            FlowState::Failed(message) => json!({"status":"failed","message":message}),
        }),
        None => None,
    });
    match result {
        Some(value) => {
            let terminal = value.get("status").and_then(Value::as_str) == Some("ready")
                || value.get("status").and_then(Value::as_str) == Some("failed");
            if terminal {
                close_flow(flow_id);
            }
            Ok(value)
        }
        None => Ok(json!({"status":"failed","message":"Luot dang nhap nay da het han hoac khong ton tai, bam Dang nhap lai"})),
    }
}

/// Huy 1 luot dang nhap: xoa flow + tra port.
pub fn cancel_oauth(flow_id: &str) {
    close_flow(flow_id);
}
