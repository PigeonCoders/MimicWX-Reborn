//! 类型定义与辅助函数

use std::time::Duration;
use tokio::time::Instant;

use mimicwx_atspi::NodeRef;

#[derive(Debug, Clone, serde::Serialize)]
pub enum WeChatStatus {
    NotRunning,
    WaitingForLogin,
    LoggedIn,
}

impl std::fmt::Display for WeChatStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning => write!(f, "未运行"),
            Self::WaitingForLogin => write!(f, "等待扫码登录"),
            Self::LoggedIn => write!(f, "已登录"),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionInfo {
    pub name: String,
    pub has_new: bool,
}

pub(crate) struct CachedNode {
    node: NodeRef,
    cached_at: Instant,
}

impl CachedNode {
    pub(crate) fn new(node: NodeRef) -> Self {
        Self { node, cached_at: Instant::now() }
    }

    pub(crate) fn get(&self, ttl_secs: u64) -> Option<&NodeRef> {
        if self.cached_at.elapsed() < Duration::from_secs(ttl_secs) {
            Some(&self.node)
        } else {
            None
        }
    }
}

pub(crate) fn is_wechat(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("wechat") || lower.contains("weixin") || name.contains("微信")
}

pub(crate) fn is_wechat_main(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == "wechat" || lower == "weixin" || name == "微信"
}
