use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub struct HeartbeatState {
    pub running: AtomicBool,
    pub cancel: Mutex<Option<Arc<AtomicBool>>>,
    pub start_lock: Mutex<()>,
}
