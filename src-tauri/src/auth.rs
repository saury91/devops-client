use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

fn http_client() -> Result<Client, String> {
    Client::builder()
        .danger_accept_invalid_certs(false)
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub fingerprint: String,
    #[serde(rename = "deviceName")]
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub code: i32,
    pub msg: String,
    pub data: Option<LoginData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginData {
    pub status: Option<String>,
    pub token: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStatusResponse {
    pub code: i32,
    pub msg: Option<String>,
    pub data: Option<DeviceStatusData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStatusData {
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceStatus {
    Active,
    Pending,
    Revoked,
    NotFound,
    Error(String),
}

/// Structured error from login_device: categorizes failures so the frontend can
/// show appropriate UI for each case.
#[derive(Debug, Clone)]
pub enum LoginError {
    Network(String),
    Server(i32, String),
}

impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoginError::Network(detail) => write!(f, "NETWORK: {}", detail),
            LoginError::Server(code, msg) => write!(f, "SERVER[{}]: {}", code, msg),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn login_device(
    server_url: &str,
    username: &str,
    password: &str,
    fingerprint: &str,
    device_name: &str,
    os: &str,
    os_version: &str,
    client_version: &str,
    device_info: &str,
) -> Result<LoginResponse, LoginError> {
    let client = http_client().map_err(|e| LoginError::Network(e))?;

    let url = format!("{}/api/auth/login-device", server_url.trim_end_matches('/'));

    let req = serde_json::json!({
        "username": username,
        "password": password,
        "fingerprint": fingerprint,
        "deviceName": device_name,
        "os": os,
        "osVersion": os_version,
        "clientVersion": client_version,
        "deviceInfo": device_info
    });

    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|e| LoginError::Network(format!("connect failed: {}", e)))?;

    let result: LoginResponse = resp
        .json()
        .await
        .map_err(|e| LoginError::Network(format!("parse response failed: {}", e)))?;

    Ok(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfoResponse {
    pub code: i32,
    pub msg: String,
    pub data: Option<UserInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: Option<i64>,
    pub username: Option<String>,
    pub nickname: Option<String>,
    pub avatar: Option<String>,
}

pub async fn get_user_info(server_url: &str, token: &str) -> Result<UserInfoResponse, String> {
    let client = http_client()?;

    let url = format!("{}/api/auth/get-user-info", server_url.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .header("X-Session-Id", token)
        .send()
        .await
        .map_err(|e| format!("get_user_info: connect failed: {}", e))?;

    let result: UserInfoResponse = resp
        .json()
        .await
        .map_err(|e| format!("get_user_info: parse response failed: {}", e))?;

    Ok(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePasswordResponse {
    pub code: i32,
    pub msg: String,
}

/// 修改当前登录用户密码；成功后服务端会强制该用户所有会话下线。
pub async fn change_password(
    server_url: &str,
    token: &str,
    old_password: &str,
    new_password: &str,
) -> Result<(), String> {
    let client = http_client()?;
    let url = format!(
        "{}/api/auth/change-password",
        server_url.trim_end_matches('/')
    );

    let body = serde_json::json!({
        "oldPassword": old_password,
        "newPassword": new_password,
    });

    let resp = client
        .post(&url)
        .header("X-Session-Id", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("change_password: connect failed: {}", e))?;

    let result: ChangePasswordResponse = resp
        .json()
        .await
        .map_err(|e| format!("change_password: parse response failed: {}", e))?;

    if result.code != 200 {
        return Err(result.msg);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeTokenResponse {
    pub code: i32,
    pub msg: String,
    pub data: Option<ExchangeTokenData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeTokenData {
    #[serde(rename = "exchangeToken")]
    pub exchange_token: Option<String>,
}

pub async fn create_exchange_token(server_url: &str, token: &str) -> Result<String, String> {
    let client = http_client()?;

    let url = format!(
        "{}/api/auth/create-exchange-token",
        server_url.trim_end_matches('/')
    );

    let resp = client
        .post(&url)
        .header("X-Session-Id", token)
        .send()
        .await
        .map_err(|e| format!("create_exchange_token: connect failed: {}", e))?;

    let result: ExchangeTokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("create_exchange_token: parse response failed: {}", e))?;

    if result.code != 200 {
        return Err(result.msg);
    }

    result
        .data
        .and_then(|d| d.exchange_token)
        .ok_or_else(|| "exchange token is empty".to_string())
}

pub async fn auto_login(server_url: &str, fingerprint: &str) -> Result<LoginResponse, String> {
    let client = http_client()?;

    let url = format!("{}/api/auth/auto-login", server_url.trim_end_matches('/'));

    let resp = client
        .post(&url)
        .form(&[("fingerprint", fingerprint)])
        .send()
        .await
        .map_err(|e| format!("auto_login: connect failed: {}", e))?;

    let result: LoginResponse = resp
        .json()
        .await
        .map_err(|e| format!("auto_login: parse response failed: {}", e))?;

    Ok(result)
}

pub async fn check_device_status(
    server_url: &str,
    fingerprint: &str,
    token: &str,
) -> Result<DeviceStatus, String> {
    let client = http_client()?;

    let url = format!(
        "{}/api/auth/device-status",
        server_url.trim_end_matches('/')
    );

    let resp = client
        .post(&url)
        .header("X-Session-Id", token)
        .json(&serde_json::json!({ "fingerprint": fingerprint }))
        .send()
        .await
        .map_err(|_| "Failed to connect".to_string())?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Ok(DeviceStatus::Error("SESSION_INVALID".to_string()));
    }
    if status == reqwest::StatusCode::FORBIDDEN {
        return Ok(DeviceStatus::Error("FINGERPRINT_MISMATCH".to_string()));
    }

    let result: DeviceStatusResponse = resp
        .json()
        .await
        .map_err(|_| "Failed to parse response".to_string())?;

    if result.code != 200 {
        let reason = match result.code {
            401 => "SESSION_INVALID".to_string(),
            403 => "FINGERPRINT_MISMATCH".to_string(),
            _ => result.msg.unwrap_or_else(|| "Unknown error".to_string()),
        };
        return Ok(DeviceStatus::Error(reason));
    }

    match result.data.and_then(|d| d.status) {
        Some(s) => match s.as_str() {
            "active" => Ok(DeviceStatus::Active),
            "pending" => Ok(DeviceStatus::Pending),
            "revoked" => Ok(DeviceStatus::Revoked),
            "not_found" => Ok(DeviceStatus::NotFound),
            _ => Ok(DeviceStatus::Error(format!("Unknown status: {}", s))),
        },
        None => Ok(DeviceStatus::NotFound),
    }
}
