//! 独立窗口监听管理

use anyhow::Result;
use tracing::{debug, info, warn};

use mimicwx_atspi::{NodeRef, registry};
use mimicwx_atspi::helpers::{wait_for, wait_for_result};
use mimicwx_core::ms;
use mimicwx_input::InputDevice;

use crate::chatwnd::ChatWnd;
use crate::types::is_wechat_main;
use crate::WeChat;

impl WeChat {
    pub async fn add_listen(
        &self,
        engine: &mut dyn InputDevice,
        who: &str,
    ) -> Result<bool> {
        info!(target: "mimicwx::listen", "添加监听: {who}");

        let app = self.find_app().await
            .ok_or_else(|| anyhow::anyhow!("找不到微信应用"))?;

        {
            let mut windows = self.listen_windows.lock().await;
            if let Some(chatwnd) = windows.get(who) {
                if chatwnd.is_alive().await {
                    debug!(target: "mimicwx::listen", "窗口已存在且存活: {who}");
                    return Ok(true);
                } else {
                    debug!(target: "mimicwx::listen", "窗口已失效, 移除旧记录: {who}");
                    windows.remove(who);
                }
            }
        }

        if let Some(wnd_node) = self.find_chat_window(&app, who).await {
            let mut windows = self.listen_windows.lock().await;
            let mut chatwnd = ChatWnd::new(who.to_string(), self.atspi.clone(), wnd_node);
            chatwnd.init_edit_box().await;
            chatwnd.init_msg_list().await;
            windows.insert(who.to_string(), chatwnd);
            debug!(target: "mimicwx::listen", "已注册现有窗口: {who}");
            return Ok(true);
        }

        for attempt in 0..3u32 {
            if attempt > 0 {
                info!(target: "mimicwx::listen", "重试添加监听 ({}/3): {who}", attempt + 1);
                tokio::time::sleep(ms((1000 + attempt * 500).into())).await;
            }

            self.focus_main_window(engine).await;

            if let Err(e) = self.chat_with(engine, who).await {
                warn!(target: "mimicwx::listen", "chat_with 失败 ({}/3): {e}", attempt + 1);
                continue;
            }

            let double_clicked = if let Some(list) = self.find_session_list(&app).await {
                if let Some(item) = self.find_session(&list, who).await {
                    if let Some(bbox) = self.atspi.bbox(&item).await {
                        let (cx, cy) = bbox.center();
                        engine.double_click(cx, cy).await?;
                        debug!(target: "mimicwx::listen", "双击会话: ({cx}, {cy})");
                        true
                    } else { false }
                } else {
                    warn!(target: "mimicwx::listen", "未找到会话项: {who} ({}/3)", attempt + 1);
                    false
                }
            } else {
                warn!(target: "mimicwx::listen", "未找到会话列表 ({}/3)", attempt + 1);
                false
            };

            if !double_clicked {
                continue;
            }

            let appeared = wait_for(&self.atspi, &app, 3000, 100,
                |atspi, app| {
                    let atspi = atspi.clone();
                    let app = app.clone();
                    let who_owned = who.to_string();
                    async move {
                        let count = atspi.child_count(&app).await;
                        for i in 0..count.min(20) {
                            if let Some(child) = atspi.child_at(&app, i).await {
                                let role = atspi.role(&child).await;
                                let name = atspi.name(&child).await;
                                if role == "frame" && name.contains(&who_owned) && !is_wechat_main(&name) {
                                    return true;
                                }
                            }
                        }
                        false
                    }
                }
            ).await;
            debug!(target: "mimicwx::listen", "窗口弹出: {}", if appeared { "已检测" } else { "超时" });
            *self.current_chat.lock().await = None;

            let wnd_node = wait_for_result(&self.atspi, &app, 6000, 200,
                |atspi, app| {
                    let atspi = atspi.clone();
                    let app = app.clone();
                    let who_owned = who.to_string();
                    async move {
                        let count = atspi.child_count(&app).await;
                        for i in 0..count.min(20) {
                            if let Some(child) = atspi.child_at(&app, i).await {
                                let role = atspi.role(&child).await;
                                let name = atspi.name(&child).await;
                                if role == "frame" && name.contains(&who_owned) && !is_wechat_main(&name) {
                                    return Some(child);
                                }
                            }
                        }
                        None
                    }
                }
            ).await;

            if let Some(wnd_node) = wnd_node {
                let mut chatwnd = ChatWnd::new(who.to_string(), self.atspi.clone(), wnd_node);
                chatwnd.init_edit_box().await;
                chatwnd.init_msg_list().await;
                let mut windows = self.listen_windows.lock().await;
                windows.insert(who.to_string(), chatwnd);
                info!(target: "mimicwx::listen", "添加成功: {who}");
                return Ok(true);
            }

            warn!(target: "mimicwx::listen", "双击未弹出窗口 ({}/3), {}", attempt + 1,
                if attempt < 2 { "将重试..." } else { "已达最大重试次数" });
        }

        warn!(target: "mimicwx::listen", "添加监听失败 (3次重试): {who}");
        Ok(false)
    }

    pub async fn remove_listen(&self, engine: &dyn InputDevice, who: &str) -> bool {
        let mut windows = self.listen_windows.lock().await;
        if windows.remove(who).is_some() {
            info!(target: "mimicwx::listen", "移除监听: {who}");
            drop(windows);
            match engine.close_window_by_title(who) {
                Ok(true) => info!(target: "mimicwx::listen", "已关闭窗口: {who}"),
                Ok(false) => info!(target: "mimicwx::listen", "窗口已关闭: {who}"),
                Err(e) => warn!(target: "mimicwx::listen", "X11 关闭窗口失败: {e}"),
            }
            *self.current_chat.lock().await = None;
            true
        } else {
            false
        }
    }

    pub async fn get_listen_list(&self) -> Vec<String> {
        let windows = self.listen_windows.lock().await;
        windows.keys().cloned().collect()
    }

    async fn find_chat_window(&self, app: &NodeRef, who: &str) -> Option<NodeRef> {
        let app_child_count = self.atspi.child_count(app).await;
        for i in 0..app_child_count.min(20) {
            if let Some(child) = self.atspi.child_at(app, i).await {
                let role = self.atspi.role(&child).await;
                let name = self.atspi.name(&child).await;
                if role == "frame" && name.contains(who) && !is_wechat_main(&name) {
                    debug!(target: "mimicwx::wechat", "找到独立窗口 (app): {name}");
                    return Some(child);
                }
            }
        }

        if let Some(reg) = registry() {
            let count = self.atspi.child_count(&reg).await;
            for i in 0..count {
                if let Some(child) = self.atspi.child_at(&reg, i).await {
                    let name = self.atspi.name(&child).await;
                    if name.contains(who) && !is_wechat_main(&name) {
                        let child_count = self.atspi.child_count(&child).await;
                        for j in 0..child_count.min(5) {
                            if let Some(frame) = self.atspi.child_at(&child, j).await {
                                let role = self.atspi.role(&frame).await;
                                if role == "frame" {
                                    let fname = self.atspi.name(&frame).await;
                                    if fname.contains(who) {
                                        debug!(target: "mimicwx::wechat", "找到独立窗口 (registry): {fname}");
                                        return Some(frame);
                                    }
                                }
                            }
                        }
                        let role = self.atspi.role(&child).await;
                        debug!(target: "mimicwx::wechat", "跳过非精确匹配: [{role}] {name}");
                    }
                }
            }
        }
        None
    }

    pub async fn check_listen_window(&self, to: &str) -> bool {
        let window_node = {
            let windows = self.listen_windows.lock().await;
            match windows.get(to) {
                Some(chatwnd) => Some(chatwnd.window_node.clone()),
                None => None,
            }
        };

        let node = match window_node {
            Some(n) => n,
            None => return false,
        };

        let alive = if let Some(bbox) = self.atspi.bbox(&node).await {
            bbox.w > 0 && bbox.h > 0
        } else {
            false
        };

        if alive {
            return true;
        }

        debug!(target: "mimicwx::send", "独立窗口已失效, 移除: {to}");
        let mut windows = self.listen_windows.lock().await;
        windows.remove(to);
        drop(windows);
        *self.current_chat.lock().await = None;
        false
    }

    pub async fn try_recover_listen_window(
        &self,
        engine: &mut dyn InputDevice,
        to: &str,
    ) -> bool {
        let has_window = self.listen_windows.lock().await.contains_key(to);
        if has_window {
            return true;
        }
        info!(target: "mimicwx::listen", "尝试恢复窗口: {to}");
        match self.add_listen(engine, to).await {
            Ok(true) => {
                info!(target: "mimicwx::listen", "窗口恢复成功: {to}");
                true
            }
            Ok(false) => {
                warn!(target: "mimicwx::listen", "窗口恢复失败: {to}");
                false
            }
            Err(e) => {
                warn!(target: "mimicwx::listen", "窗口恢复出错: {to} — {e}");
                false
            }
        }
    }
}
