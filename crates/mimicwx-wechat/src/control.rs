//! 控件查找 + 会话列表

use tracing::{debug, warn};

use mimicwx_atspi::{NodeRef, SearchAction};
use mimicwx_atspi::search;
use mimicwx_core::{is_message_list, is_structural_role, ms};
use mimicwx_input::InputDevice;

use crate::types::{is_wechat_main, CachedNode, SessionInfo};
use crate::WeChat;

impl WeChat {
    pub async fn find_split_pane(&self, app: &NodeRef) -> Option<NodeRef> {
        search::find_bfs(self.atspi.as_ref(), app, |role, _| {
            role == "splitter" || role == "split pane"
        }).await
    }

    pub async fn find_session_list(&self, app: &NodeRef) -> Option<NodeRef> {
        {
            let cache = self.cached_session_list.lock().await;
            if let Some(ref cached) = *cache {
                if let Some(node) = cached.get(10) {
                    return Some(node.clone());
                }
            }
        }

        let result = search::find_dfs(
            self.atspi.as_ref(),
            app,
            &|role, name| {
                if role == "list" && (name.contains("Chats") || name.contains("会话")) {
                    SearchAction::Found
                } else {
                    SearchAction::Recurse
                }
            },
            0, 18, 20,
        ).await;
        if let Some(ref node) = result {
            debug!(target: "mimicwx::wechat", "找到会话列表");
            *self.cached_session_list.lock().await = Some(CachedNode::new(node.clone()));
        }
        result
    }

    pub async fn find_message_list(&self, app: &NodeRef) -> Option<NodeRef> {
        let result = search::find_dfs(
            self.atspi.as_ref(),
            app,
            &is_message_list,
            0, 18, 20,
        ).await;
        if result.is_some() {
            debug!(target: "mimicwx::wechat", "找到消息列表");
        }
        result
    }

    pub async fn find_edit_box(&self, app: &NodeRef) -> Option<NodeRef> {
        search::find_dfs(
            self.atspi.as_ref(),
            app,
            &|role, _| {
                if role == "entry" || role == "text" {
                    SearchAction::Found
                } else {
                    SearchAction::Recurse
                }
            },
            0, 18, 20,
        ).await
    }

    pub async fn find_session(&self, container: &NodeRef, name: &str) -> Option<NodeRef> {
        let mut best_starts_with: Option<NodeRef> = None;
        let mut best_contains: Option<NodeRef> = None;

        let mut frontier = vec![container.clone()];
        for _depth in 0..6 {
            if frontier.is_empty() { break; }
            let mut next = Vec::new();
            for node in &frontier {
                let count = self.atspi.child_count(node).await;
                for i in 0..count.min(30) {
                    if let Some(child) = self.atspi.child_at(node, i).await {
                        let item_name = self.atspi.name(&child).await;
                        let trimmed = item_name.trim();
                        if !trimmed.is_empty() {
                            if trimmed == name {
                                return Some(child);
                            }
                            if best_starts_with.is_none() && trimmed.starts_with(name) {
                                best_starts_with = Some(child.clone());
                            } else if best_contains.is_none() && trimmed.contains(name) {
                                best_contains = Some(child.clone());
                            }
                        }
                        let role = self.atspi.role(&child).await;
                        if is_structural_role(&role) {
                            next.push(child);
                        }
                    }
                }
            }
            frontier = next;
        }
        best_starts_with.or(best_contains)
    }

    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        let app = match self.find_app().await {
            Some(a) => a,
            None => return Vec::new(),
        };
        let list = match self.find_session_list(&app).await {
            Some(l) => l,
            None => return Vec::new(),
        };

        let count = self.atspi.child_count(&list).await;
        let mut sessions = Vec::new();

        for i in 0..count.min(50) {
            if let Some(child) = self.atspi.child_at(&list, i).await {
                let name = self.atspi.name(&child).await;
                let trimmed = name.trim().to_string();
                if trimmed.len() > 1 {
                    let has_new = self.check_session_has_new(&child).await;
                    sessions.push(SessionInfo { name: trimmed, has_new });
                }
            }
        }

        sessions
    }

    async fn check_session_has_new(&self, session: &NodeRef) -> bool {
        let count = self.atspi.child_count(session).await;
        for i in 0..count.min(10) {
            if let Some(child) = self.atspi.child_at(session, i).await {
                let role = self.atspi.role(&child).await;
                let name = self.atspi.name(&child).await;
                if (role == "label" || role == "static")
                    && !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_digit())
                {
                    return true;
                }
            }
        }
        false
    }

    pub(crate) async fn focus_main_window(&self, engine: &mut dyn InputDevice) {
        for title in ["微信", "WeChat", "Weixin"] {
            match engine.activate_window_by_title(title, true) {
                Ok(true) => {
                    debug!(target: "mimicwx::wechat", "激活主窗口: {title}");
                    tokio::time::sleep(ms(300)).await;
                    return;
                }
                Ok(false) => {}
                Err(e) => debug!(target: "mimicwx::wechat", "X11 激活失败: {e}"),
            }
        }

        if let Some(app) = self.find_app().await {
            let count = self.atspi.child_count(&app).await;
            for i in 0..count.min(10) {
                if let Some(child) = self.atspi.child_at(&app, i).await {
                    let role = self.atspi.role(&child).await;
                    let name = self.atspi.name(&child).await;
                    if role == "frame" && is_wechat_main(&name) {
                        if let Some(bbox) = self.atspi.bbox(&child).await {
                            let cx = (bbox.x + bbox.w / 2).max(0);
                            let cy = (bbox.y + 15).max(0);
                            debug!(target: "mimicwx::wechat", "AT-SPI 点击主窗口: ({cx}, {cy})");
                            let _ = engine.click(cx, cy).await;
                            tokio::time::sleep(ms(300)).await;
                            return;
                        }
                    }
                }
            }
        }
        warn!(target: "mimicwx::wechat", "无法聚焦主窗口");
    }
}
