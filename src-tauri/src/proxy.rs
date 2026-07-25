use axum::{http::header, response::IntoResponse, routing::get, Router};
use serde_json::json;
use std::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

async fn ping() -> impl IntoResponse {
    let body = json!({ "status": "ok" }).to_string();
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body,
    )
}

async fn serve(
    listener: std::net::TcpListener,
    shutdown_rx: oneshot::Receiver<()>,
) -> Result<(), String> {
    let app = Router::new().route("/ping", get(ping));

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
pub fn start_proxy(_fingerprint: String) -> Result<(u16, oneshot::Sender<()>), String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Failed to bind: {}", e))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (tx, rx) = oneshot::channel();

    std::thread::spawn(move || {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let _ = serve(listener, rx).await;
        });
    });

    Ok((port, tx))
}
