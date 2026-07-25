use reqwest::Client;
use serde::{Deserialize, Serialize};

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
) -> Result<LoginResponse, String> {
    let client = Client::builder()
        .danger_accept_invalid_certs(false)
        .build()
        .map_err(|e| e.to_string())?;

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
        .map_err(|e| format!("连接失败: {}", e))?;

    let result: LoginResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

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
    let client = Client::builder()
        .danger_accept_invalid_certs(false)
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("{}/users/me", server_url.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .header("X-Session-Id", token)
        .send()
        .await
        .map_err(|e| format!("获取用户信息失败: {}", e))?;

    let result: UserInfoResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析用户信息失败: {}", e))?;

    Ok(result)
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
    let client = Client::builder()
        .danger_accept_invalid_certs(false)
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!(
        "{}/api/auth/create-exchange-token",
        server_url.trim_end_matches('/')
    );

    let resp = client
        .post(&url)
        .header("X-Session-Id", token)
        .send()
        .await
        .map_err(|e| format!("创建兑换令牌失败: {}", e))?;

    let result: ExchangeTokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析兑换令牌响应失败: {}", e))?;

    if result.code != 200 {
        return Err(result.msg);
    }

    result
        .data
        .and_then(|d| d.exchange_token)
        .ok_or_else(|| "兑换令牌为空".to_string())
}

pub async fn auto_login(server_url: &str, fingerprint: &str) -> Result<LoginResponse, String> {
    let client = Client::builder()
        .danger_accept_invalid_certs(false)
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("{}/api/auth/auto-login", server_url.trim_end_matches('/'));

    let resp = client
        .post(&url)
        .form(&[("fingerprint", fingerprint)])
        .send()
        .await
        .map_err(|e| format!("自动登录请求失败: {}", e))?;

    let result: LoginResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析自动登录响应失败: {}", e))?;

    Ok(result)
}

pub async fn check_device_status(
    server_url: &str,
    fingerprint: &str,
    token: &str,
) -> Result<DeviceStatus, String> {
    let client = Client::builder()
        .danger_accept_invalid_certs(false)
        .build()
        .map_err(|e| e.to_string())?;

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
