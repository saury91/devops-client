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

/// 两次 `proxy-ping` 事件之间的最小间隔。
///
/// 工作台的存活检查是 3 秒一轮，这里取 1 秒，等于"每轮至多通知一次"；同时挡住页面刷新
/// 或重试造成的突发请求 —— 每个请求都往窗口发一次事件会让前端反复重渲染。
const PING_EMIT_INTERVAL_MS: u64 = 1000;

/// 代理与宿主应用之间的通知口。
///
/// 抽成 trait 对象而不是直接持有 `AppHandle`，是为了让路由能在单元测试里构造：
/// `AppHandle` 必须有真实的 Tauri 应用才能创建，而这个模块值得断言的恰好是 HTTP 层的
/// 契约（状态码、CORS 头、缓存头、通知节流），它们都不需要一个真窗口。
trait Pinger: Send + Sync {
    /// 告知宿主：工作台刚刚探活。
    fn ping(&self);
}

impl Pinger for AppHandle {
    fn ping(&self) {
        let _ = self.emit("proxy-ping", ());
    }
}

struct ProxyContext {
    pinger: Box<dyn Pinger>,
    last_emit_ms: AtomicU64,
}

/// `/ping` 的响应头。
///
/// `Cache-Control: no-store` 不是可选项：这个端点回答的是"本机客户端此刻还在不在"。
/// 任何一层缓存（浏览器、系统代理）把这个 200 留上几分钟，工作台就会在一个已经退出的
/// 客户端上显示"在线" —— 而这正是该端点唯一要保证的事。
fn probe_headers(builder: axum::http::response::Builder) -> axum::http::response::Builder {
    builder
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, OPTIONS")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Content-Type")
        .header(header::CACHE_CONTROL, "no-store")
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
        ctx.pinger.ping();
    }

    let body = json!({ "status": "ok" }).to_string();
    probe_headers(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json; charset=utf-8"),
    )
    .body(Body::from(body))
    .unwrap_or_else(|_| Response::new(Body::empty()))
}

async fn handle_options() -> Response<Body> {
    probe_headers(Response::builder().status(StatusCode::NO_CONTENT))
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// 从 `serve` 里拆出来的路由，让测试不必依赖一个真实的 Tauri 应用。
fn router(ctx: Arc<ProxyContext>) -> Router {
    Router::new()
        .route("/ping", get(ping).options(handle_options))
        .with_state(ctx)
}

async fn serve(
    listener: std::net::TcpListener,
    shutdown_rx: oneshot::Receiver<()>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let ctx = Arc::new(ProxyContext {
        pinger: Box::new(app_handle),
        last_emit_ms: AtomicU64::new(0),
    });

    let tokio_listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|e| format!("Failed to convert listener: {}", e))?;

    axum::serve(tokio_listener, router(ctx).into_make_service())
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .await
        .map_err(|e| format!("Server error: {}", e))
}

/// Start HTTP proxy on OS-assigned port. Returns (port, shutdown_sender).
///
/// `_fingerprint` 由调用方（前端 invoke 的既有参数）透传进来，代理本身用不到它：
/// 它监听的是本机 127.0.0.1 上的 /ping，不代表任何设备身份。
pub fn start_proxy(
    _fingerprint: String,
    app_handle: AppHandle,
) -> Result<(u16, oneshot::Sender<()>), String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Failed to bind: {}", e))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (tx, rx) = oneshot::channel();

    // 运行时在返回之前建好：放在子线程里建，失败时 `start_proxy` 已经带着端口返回成功，
    // 状态里记着一个端口却没有任何东西在监听，界面会一直显示"代理已启动"。
    let rt = Runtime::new().map_err(|e| format!("Failed to create runtime: {}", e))?;

    std::thread::spawn(move || {
        rt.block_on(async {
            if let Err(e) = serve(listener, rx, app_handle).await {
                // 用日志而不是 eprintln：Windows 上没有控制台，stderr 写完等于没写，
                // 而这是"代理停止服务"这类必须留痕的事件。
                crate::config::log_error("proxy", &format!("server stopped: {}", e));
            }
        });
    });

    Ok((port, tx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// 记录被通知了几次，让测试能断言节流确实生效。
    #[derive(Clone, Default)]
    struct CountingPinger(Arc<AtomicUsize>);

    impl Pinger for CountingPinger {
        fn ping(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 在随机端口上起一个真实的服务实例，返回 (base URL, 通知计数)。
    ///
    /// 走真实端口而不是 `tower::ServiceExt::oneshot`：这里要验证的东西（CORS 预检、
    /// 缓存头、多次请求之间的节流）全都依赖真实连接往返，直接调 handler 会把它们绕过去。
    fn spawn() -> (String, Arc<AtomicUsize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("listener address").port();
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");

        let hits = Arc::new(AtomicUsize::new(0));
        let ctx = Arc::new(ProxyContext {
            pinger: Box::new(CountingPinger(hits.clone())),
            last_emit_ms: AtomicU64::new(0),
        });

        let tokio_listener =
            tokio::net::TcpListener::from_std(listener).expect("convert to tokio listener");
        tokio::spawn(async move {
            let _ = axum::serve(tokio_listener, router(ctx).into_make_service()).await;
        });

        (format!("http://127.0.0.1:{}", port), hits)
    }

    fn header_value(resp: &reqwest::Response, name: header::HeaderName) -> Option<String> {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    }

    #[tokio::test]
    async fn ping_answers_json_and_forbids_caching() {
        let (base, _hits) = spawn();

        let resp = reqwest::get(format!("{}/ping", base))
            .await
            .expect("ping must answer");
        assert_eq!(resp.status(), 200);
        // 没有这个头，工作台的跨源轮询会被浏览器直接拦掉。
        assert_eq!(
            header_value(&resp, header::ACCESS_CONTROL_ALLOW_ORIGIN).as_deref(),
            Some("*")
        );
        // 被缓存 = 把已退出的客户端判成在线。
        assert_eq!(
            header_value(&resp, header::CACHE_CONTROL).as_deref(),
            Some("no-store")
        );
        assert_eq!(resp.text().await.expect("body"), "{\"status\":\"ok\"}");
    }

    #[tokio::test]
    async fn preflight_is_answered_without_a_body() {
        let (base, _hits) = spawn();

        let resp = reqwest::Client::new()
            .request(reqwest::Method::OPTIONS, format!("{}/ping", base))
            .send()
            .await
            .expect("preflight must answer");

        // 204 + 空体：预检只需要头，带上 body 反而会让部分客户端去解析它。
        assert_eq!(resp.status(), 204);
        assert_eq!(
            header_value(&resp, header::ACCESS_CONTROL_ALLOW_METHODS).as_deref(),
            Some("GET, OPTIONS")
        );
        assert_eq!(
            header_value(&resp, header::CACHE_CONTROL).as_deref(),
            Some("no-store")
        );
        assert_eq!(resp.bytes().await.expect("body").len(), 0);
    }

    #[tokio::test]
    async fn burst_of_pings_notifies_the_host_only_once() {
        let (base, hits) = spawn();
        let client = reqwest::Client::new();

        for _ in 0..5 {
            let resp = client
                .get(format!("{}/ping", base))
                .send()
                .await
                .expect("ping must answer");
            assert_eq!(resp.status(), 200);
        }

        // 5 次探活落在同一个 1 秒窗口内，宿主只应被通知一次 —— 否则每次轮询都会让
        // 前端重渲染，而工作台本身是 3 秒一轮。
        assert_eq!(hits.load(Ordering::Relaxed), 1);
    }
}
