//! WeChat 主结构体

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use tokio::sync::Mutex;

use mimicwx_atspi::AtSpi;

use crate::chatwnd::ChatWnd;
use crate::types::CachedNode;

pub struct WeChat {
    pub(crate) atspi: Arc<AtSpi>,
    pub(crate) listen_windows: Mutex<HashMap<String, ChatWnd>>,
    pub(crate) current_chat: Mutex<Option<String>>,
    pub(crate) at_delay_ms: AtomicU64,
    pub(crate) cached_app: Mutex<Option<CachedNode>>,
    pub(crate) cached_session_list: Mutex<Option<CachedNode>>,
}

impl WeChat {
    pub fn new(atspi: Arc<AtSpi>, at_delay_ms: u64) -> Self {
        Self {
            atspi,
            listen_windows: Mutex::new(HashMap::new()),
            current_chat: Mutex::new(None),
            at_delay_ms: AtomicU64::new(at_delay_ms),
            cached_app: Mutex::new(None),
            cached_session_list: Mutex::new(None),
        }
    }

    pub fn get_at_delay_ms(&self) -> u64 {
        self.at_delay_ms.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_at_delay_ms(&self, ms: u64) {
        self.at_delay_ms.store(ms, std::sync::atomic::Ordering::Relaxed);
    }
}
