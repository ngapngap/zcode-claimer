//! AutoClaw desktop Electron safeStorage decryption (ported from desktop protocol).
//! DPAPI (Windows only) + AES-256-GCM for token/refreshToken enc: fields in
//! %APPDATA%/AutoClaw/auth.json. Plaintext (non-enc:) values pass through.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::DecodePaddingMode;
use base64::Engine as _;

const OS_CRYPT_VERSION: &str = "v10";
const ENC_PREFIX: &str = "enc:";
const DPAPI_PREFIX_LEN: usize = 5;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const AES_KEY_LEN: usize = 32;

const OS_CRYPT_BASE64: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

fn aes_key_cache() -> &'static Mutex<Option<(String, Vec<u8>)>> {
    static CACHE: OnceLock<Mutex<Option<(String, Vec<u8>)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

pub fn os_crypt_aes_key(local_state_path: &Path) -> Result<Vec<u8>, String> {
    let cache_key = local_state_path.to_string_lossy().to_string();
    if let Ok(guard) = aes_key_cache().lock() {
        if let Some((cached_path, key)) = guard.as_ref() {
            if *cached_path == cache_key {
                return Ok(key.clone());
            }
        }
    }
    let key = load_os_crypt_aes_key(local_state_path)?;
    if let Ok(mut guard) = aes_key_cache().lock() {
        *guard = Some((cache_key, key.clone()));
    }
    Ok(key)
}

fn load_os_crypt_aes_key(local_state_path: &Path) -> Result<Vec<u8>, String> {
    let text = std::fs::read_to_string(local_state_path).map_err(|error| {
        format!(
            "Khong doc duoc AutoClaw Local State ({}): {}",
            local_state_path.display(),
            error
        )
    })?;
    let value = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|_| "AutoClaw Local State khong phai JSON hop le".to_string())?;
    let encoded = value
        .get("os_crypt")
        .and_then(|o| o.get("encrypted_key"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    if encoded.is_empty() {
        return Err("AutoClaw Local State thieu os_crypt.encrypted_key".to_string());
    }
    if !is_base64_like(encoded) {
        return Err("AutoClaw os_crypt.encrypted_key khong phai base64".to_string());
    }
    let blob = OS_CRYPT_BASE64
        .decode(encoded)
        .map_err(|_| "AutoClaw os_crypt.encrypted_key giai ma that bai".to_string())?;
    if blob.len() <= DPAPI_PREFIX_LEN {
        return Err("AutoClaw os_crypt key blob qua ngan".to_string());
    }
    if &blob[..DPAPI_PREFIX_LEN] != b"DPAPI" {
        return Err("AutoClaw os_crypt key prefix khong phai DPAPI".to_string());
    }
    let key = dpapi::unprotect(&blob[DPAPI_PREFIX_LEN..])?;
    if key.len() != AES_KEY_LEN {
        return Err(format!(
            "AutoClaw os_crypt key dai {} byte (can 32)",
            key.len()
        ));
    }
    Ok(key)
}

fn is_base64_like(value: &str) -> bool {
    let mut padding = 0usize;
    let mut seen_padding = false;
    let mut body = 0usize;
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => {
                if seen_padding {
                    return false;
                }
                body += 1;
            }
            b'=' => {
                seen_padding = true;
                padding += 1;
                if padding > 2 {
                    return false;
                }
            }
            _ => return false,
        }
    }
    body > 0
}

pub fn decrypt_enc_value(value: &str, aes_key: Option<&[u8]>) -> Result<String, String> {
    let Some(encoded) = value.strip_prefix(ENC_PREFIX) else {
        return Ok(value.to_string());
    };
    let Some(key) = aes_key else {
        return Err("Khong co os_crypt key de giai ma enc:".to_string());
    };
    let blob = OS_CRYPT_BASE64
        .decode(encoded.trim())
        .map_err(|_| "AutoClaw enc: khong phai base64".to_string())?;
    if blob.len() < 3 {
        return Err("AutoClaw enc: qua ngan".to_string());
    }
    let prefix = String::from_utf8_lossy(&blob[..3]).to_string();
    if prefix != OS_CRYPT_VERSION {
        return Err(format!("Khong ho tro os_crypt version: {}", prefix));
    }
    let payload = &blob[3..];
    if payload.len() <= NONCE_LEN + TAG_LEN {
        return Err("os_crypt ciphertext qua ngan".to_string());
    }
    let nonce = &payload[..NONCE_LEN];
    let body = &payload[NONCE_LEN..];
    let plaintext = aes_gcm_decrypt(key, nonce, body)?;
    String::from_utf8(plaintext).map_err(|_| "Giai ma ra khong phai UTF-8".to_string())
}

fn aes_gcm_decrypt(key: &[u8], nonce: &[u8], ciphertext_with_tag: &[u8]) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::consts::U12;
    use aes_gcm::aead::Aead as _;
    use aes_gcm::{Aes256Gcm, KeyInit as _, Nonce};

    if key.len() != AES_KEY_LEN {
        return Err(format!("os_crypt key dai {} (can 32)", key.len()));
    }
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| "os_crypt key khong hop le cho AES-256".to_string())?;
    let nonce = Nonce::<U12>::try_from(nonce)
        .map_err(|_| format!("os_crypt nonce sai dai (can {})", NONCE_LEN))?;
    cipher
        .decrypt(&nonce, ciphertext_with_tag)
        .map_err(|_| "AES-GCM giai ma that bai (key hoac data khong khop)".to_string())
}

pub fn decode_jwt_claims(token: &str) -> Option<serde_json::Value> {
    let raw = strip_bearer(token);
    let payload = raw.split('.').nth(1)?;
    if payload.is_empty() {
        return None;
    }
    let mut normalized: String = payload
        .chars()
        .map(|ch| match ch {
            '-' => '+',
            '_' => '/',
            other => other,
        })
        .collect();
    let remainder = normalized.len() % 4;
    if remainder != 0 {
        normalized.push_str(&"=".repeat(4 - remainder));
    }
    let bytes = OS_CRYPT_BASE64.decode(&normalized).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let claims = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    claims.is_object().then_some(claims)
}

pub fn strip_bearer(value: &str) -> String {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() > b"bearer".len() && bytes[..b"bearer".len()].eq_ignore_ascii_case(b"bearer") {
        let rest = &trimmed[b"bearer".len()..];
        if rest.starts_with(char::is_whitespace) {
            return rest.trim_start().to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(windows)]
mod dpapi {
    use windows::core::Owned;
    use windows::Win32::Foundation::HLOCAL;
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
        if blob.is_empty() {
            return Err("DPAPI blob rong".to_string());
        }
        let input = CRYPT_INTEGER_BLOB {
            cbData: blob.len() as u32,
            pbData: blob.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe { CryptUnprotectData(&input, None, None, None, None, 0, &mut output) }.map_err(
            |error| {
                format!("DPAPI giai ma os_crypt key that bai: {}", error)
            },
        )?;
        if output.pbData.is_null() || output.cbData == 0 {
            return Err("DPAPI giai ma ra rong".to_string());
        }
        let owned = unsafe { Owned::new(HLOCAL(output.pbData as *mut _)) };
        let plaintext =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        drop(owned);
        if plaintext.is_empty() {
            return Err("DPAPI giai ma ra rong".to_string());
        }
        Ok(plaintext)
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn unprotect(_blob: &[u8]) -> Result<Vec<u8>, String> {
        Err("DPAPI chi ho tro Windows".to_string())
    }
}
