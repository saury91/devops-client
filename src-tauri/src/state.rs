use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

pub struct ProxyState {
    pub running: AtomicBool,
    pub port: Mutex<Option<u16>>,
    pub fingerprint: Mutex<String>,
    pub shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    pub start_lock: Mutex<()>,
}

pub struct HeartbeatState {
    pub running: AtomicBool,
    pub cancel: Mutex<Option<Arc<AtomicBool>>>,
    pub start_lock: Mutex<()>,
}
