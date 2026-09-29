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
    let client = http_client().map_err(LoginError::Network)?;

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

    let url = format!(
        "{}/api/auth/get-user-info",
        server_url.trim_end_matches('/')
    );

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

/// Heartbeat: asks the server for this device's status.
///
/// 心跳只负责「设备在场」这一件事：服务端据此刷新 `devops:device:alive:{fingerprint}`，
/// 设备不在场时其浏览器会话会在在场窗口过期后失效。浏览器侧另有基于不可导出密钥的
/// 设备证明心跳，两者相互独立、缺一不可。
///
/// @param fingerprint this machine's device fingerprint
/// @return the device status as recorded by the server
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, StatusCode, Uri};
    use axum::{routing::post, Json, Router};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    /// 桩服务端记录下来的「请求头/路径 → 请求体」。
    type Seen = Arc<Mutex<Vec<(String, Value)>>>;

    fn recorder() -> Seen {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn only_call(seen: &Seen) -> (String, Value) {
        let calls = seen.lock().expect("recorder lock");
        assert_eq!(calls.len(), 1, "桩服务端应当恰好被调用一次");
        calls[0].clone()
    }

    /// 在随机端口上起桩服务端，返回它的 base URL。
    ///
    /// 这里刻意用真实 HTTP，而不是把 reqwest 换成 mock：这些函数真正容易写错的地方是
    /// URL 拼接（`trim_end_matches('/')`）、请求头名（`X-Session-Id`）和 JSON 字段名
    /// （`deviceName` 这类 camelCase）。mock 掉客户端恰好会把这三类错误全部掩盖 —— 而
    /// 它们在真实环境里只表现为一句"登录失败"。
    async fn spawn_stub(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("should bind an ephemeral port");
        let addr = listener
            .local_addr()
            .expect("listener should have an address");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app.into_make_service()).await;
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn login_device_trims_trailing_slash_and_sends_camel_case_fields() {
        let seen = recorder();
        let sink = seen.clone();
        let app = Router::new().route(
            "/api/auth/login-device",
            post(move |uri: Uri, Json(body): Json<Value>| {
                let sink = sink.clone();
                async move {
                    sink.lock()
                        .expect("recorder lock")
                        .push((uri.path().to_string(), body));
                    Json(json!({
                        "code": 200,
                        "msg": "ok",
                        "data": { "status": "active", "token": "tok-123" }
                    }))
                }
            }),
        );
        let base = spawn_stub(app).await;

        // 用户在设置页填成 `https://host/` 是最常见的形式，不能拼出 `//api/auth/login-device`。
        let resp = login_device(
            &format!("{}/", base),
            "alice",
            "pw",
            "fp-1",
            "dev-1",
            "macos",
            "14.5",
            "0.1.13",
            "{}",
        )
        .await
        .expect("stub answers 200 with a token");

        assert_eq!(resp.code, 200);
        assert_eq!(resp.data.expect("data").token.as_deref(), Some("tok-123"));

        let (path, body) = only_call(&seen);
        assert_eq!(path, "/api/auth/login-device");
        assert_eq!(body["username"], "alice");
        assert_eq!(body["password"], "pw");
        assert_eq!(body["fingerprint"], "fp-1");
        assert_eq!(body["deviceName"], "dev-1");
        assert_eq!(body["os"], "macos");
        assert_eq!(body["osVersion"], "14.5");
        assert_eq!(body["clientVersion"], "0.1.13");
    }

    #[tokio::test]
    async fn login_device_reports_unreachable_server_as_network_error() {
        // 端口 1 上不可能有监听者（绑定它需要 root），连接会被立即拒绝。这比"先 bind 再
        // drop"拿到的端口更确定 —— 后者有被并行运行的其它测试抢占的可能。
        let err = login_device(
            "http://127.0.0.1:1",
            "alice",
            "pw",
            "fp",
            "dev",
            "macos",
            "14",
            "0.1.13",
            "{}",
        )
        .await
        .expect_err("connecting to a closed port must fail");

        // 传输层失败必须归到 Network：前端据此显示"连接失败"，而 Server 会显示服务端原话。
        assert!(matches!(err, LoginError::Network(_)), "got {:?}", err);
    }

    #[tokio::test]
    async fn check_device_status_sends_session_header_and_maps_status() {
        let seen = recorder();
        let sink = seen.clone();
        let app = Router::new().route(
            "/api/auth/device-status",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let sink = sink.clone();
                async move {
                    let session = headers
                        .get("X-Session-Id")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    sink.lock().expect("recorder lock").push((session, body));
                    Json(json!({ "code": 200, "msg": "ok", "data": { "status": "revoked" } }))
                }
            }),
        );
        let base = spawn_stub(app).await;

        let status = check_device_status(&base, "fp-1", "sess-9")
            .await
            .expect("stub answers 200");
        assert_eq!(status, DeviceStatus::Revoked);

        let (session, body) = only_call(&seen);
        // 头部名拼错（比如写成 X-Session-Token）在服务端只表现为"会话无效"，只有这里拦得住。
        assert_eq!(session, "sess-9");
        assert_eq!(body["fingerprint"], "fp-1");
    }

    #[tokio::test]
    async fn check_device_status_treats_http_401_as_invalid_session() {
        let app = Router::new().route(
            "/api/auth/device-status",
            post(|| async { (StatusCode::UNAUTHORIZED, "") }),
        );
        let base = spawn_stub(app).await;

        let status = check_device_status(&base, "fp", "t")
            .await
            .expect("401 is a mapped result, not a transport failure");
        // 心跳线程靠这个哨兵值触发被动登出，不能退化成通用的 Error("Failed to parse ...")。
        assert_eq!(status, DeviceStatus::Error("SESSION_INVALID".to_string()));
    }

    #[tokio::test]
    async fn change_password_surfaces_server_message_on_failure() {
        let app = Router::new().route(
            "/api/auth/change-password",
            post(|| async { Json(json!({ "code": 400, "msg": "weak password" })) }),
        );
        let base = spawn_stub(app).await;

        let err = change_password(&base, "t", "old", "new")
            .await
            .expect_err("code != 200 must be an error");
        // 直接把服务端原话透给用户，而不是包成"操作失败"。
        assert_eq!(err, "weak password");
    }

    #[tokio::test]
    async fn change_password_succeeds_on_code_200() {
        let app = Router::new().route(
            "/api/auth/change-password",
            post(|| async { Json(json!({ "code": 200, "msg": "ok" })) }),
        );
        let base = spawn_stub(app).await;

        change_password(&base, "t", "old", "new")
            .await
            .expect("code 200 must be Ok");
    }

    #[tokio::test]
    async fn create_exchange_token_rejects_ok_response_without_token() {
        let app = Router::new().route(
            "/api/auth/create-exchange-token",
            post(|| async { Json(json!({ "code": 200, "msg": "ok", "data": {} })) }),
        );
        let base = spawn_stub(app).await;

        // 交换 token 缺失时必须报错：返回空串会让"打开工作台"带着空凭据发出去，
        // 用户看到的是工作台登录页，而不是清楚的一句失败原因。
        let err = create_exchange_token(&base, "t")
            .await
            .expect_err("missing token must be an error");
        assert_eq!(err, "exchange token is empty");
    }
}
