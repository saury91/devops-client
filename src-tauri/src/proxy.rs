use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use serde_json::json;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

const PING_EMIT_INTERVAL_MS: u64 = 1000;

struct ProxyContext {
    app_handle: AppHandle,
    last_emit_ms: AtomicU64,
}

fn add_cors_headers(builder: axum::http::response::Builder) -> axum::http::response::Builder {
    builder
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, OPTIONS")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Content-Type")
}

async fn ping(State(ctx): State<Arc<ProxyContext>>) -> Response<Body> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last = ctx.last_emit_ms.load(Ordering::Relaxed);
    if now_ms.saturating_sub(last) >= PING_EMIT_INTERVAL_MS
        && ctx
            .last_emit_ms
            .compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        let _ = ctx.app_handle.emit("proxy-ping", ());
    }

    let body = json!({ "status": "ok" }).to_string();
    add_cors_headers(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json; charset=utf-8"),
    )
    .body(Body::from(body))
    .unwrap_or_else(|_| Response::new(Body::empty()))
}

async fn handle_options() -> Response<Body> {
    add_cors_headers(Response::builder().status(StatusCode::NO_CONTENT))
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

async fn serve(
    listener: std::net::TcpListener,
    shutdown_rx: oneshot::Receiver<()>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let ctx = Arc::new(ProxyContext {
        app_handle,
        last_emit_ms: AtomicU64::new(0),
    });
    let app = Router::new()
        .route("/ping", get(ping).options(handle_options))
        .with_state(ctx);

    let tokio_listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|e| format!("Failed to convert listener: {}", e))?;

    axum::serve(tokio_listener, app.into_make_service())
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .await
        .map_err(|e| format!("Server error: {}", e))
}

/// Start HTTP proxy on OS-assigned port. Returns (port, shutdown_sender).
pub fn start_proxy(
    _fingerprint: String,
    app_handle: AppHandle,
) -> Result<(u16, oneshot::Sender<()>), String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Failed to bind: {}", e))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (tx, rx) = oneshot::channel();

    std::thread::spawn(move || {
        let rt = match Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("proxy: failed to create tokio runtime: {}", e);
                return;
            }
        };
        rt.block_on(async {
            if let Err(e) = serve(listener, rx, app_handle).await {
                eprintln!("proxy: {}", e);
            }
        });
    });

    Ok((port, tx))
}
