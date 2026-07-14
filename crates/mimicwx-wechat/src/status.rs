//! 状态检测 + 应用查找

use tracing::debug;

use mimicwx_atspi::{registry, NodeRef};
use mimicwx_atspi::search;

use crate::types::{is_wechat, CachedNode, WeChatStatus};
use crate::WeChat;

impl WeChat {
    pub async fn check_status(&self) -> WeChatStatus {
        let app = match self.find_app().await {
            Some(a) => a,
            None => return WeChatStatus::NotRunning,
        };
        if self.find_nav_toolbar(&app).await.is_some() {
            WeChatStatus::LoggedIn
        } else {
            WeChatStatus::WaitingForLogin
        }
    }

    pub async fn try_reconnect(&self) -> bool {
        *self.cached_app.lock().await = None;
        *self.cached_session_list.lock().await = None;
        self.atspi.reconnect().await
    }

    pub async fn find_app(&self) -> Option<NodeRef> {
        {
            let cache = self.cached_app.lock().await;
            if let Some(ref cached) = *cache {
                if let Some(node) = cached.get(30) {
                    return Some(node.clone());
                }
            }
        }

        if let Some(app) = self.scan_registry().await {
            *self.cached_app.lock().await = Some(CachedNode::new(app.clone()));
            return Some(app);
        }
        debug!(target: "mimicwx::wechat", "Registry 未找到微信, 尝试重连");
        if self.atspi.reconnect().await {
            if let Some(app) = self.scan_registry().await {
                *self.cached_app.lock().await = Some(CachedNode::new(app.clone()));
                return Some(app);
            }
        }
        *self.cached_app.lock().await = None;
        None
    }

    async fn scan_registry(&self) -> Option<NodeRef> {
        let registry = registry()?;
        let count = self.atspi.child_count(&registry).await;
        debug!(target: "mimicwx::wechat", "Registry 子节点数: {count}");
        for i in 0..count {
            if let Some(child) = self.atspi.child_at(&registry, i).await {
                let name = self.atspi.name(&child).await;
                if is_wechat(&name) {
                    debug!(target: "mimicwx::wechat", "找到微信: {name}");
                    return Some(child);
                }
            }
        }
        None
    }

    pub async fn find_nav_toolbar(&self, app: &NodeRef) -> Option<NodeRef> {
        search::find_bfs(self.atspi.as_ref(), app, |role, name| {
            role == "tool bar" && (name.contains("导航") || name.contains("Navigation"))
        }).await
    }
}
