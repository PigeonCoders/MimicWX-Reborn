//! 独立聊天窗口 (ChatWnd)
//!
//! 每个独立弹出的聊天窗口拥有自己的 AT-SPI2 节点引用，
//! 可以独立读取消息和发送，互不干扰。

use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, info};

use mimicwx_atspi::{AtSpi, NodeRef, SearchAction};
use mimicwx_atspi::{search, helpers};
use mimicwx_core::{ms, match_message_list};
use mimicwx_input::InputDevice;

pub struct ChatWnd {
    pub who: String,
    atspi: Arc<AtSpi>,
    pub(crate) window_node: NodeRef,
    edit_box_node: Option<NodeRef>,
    msg_list_node: Option<NodeRef>,
}

impl ChatWnd {
    pub fn new(who: String, atspi: Arc<AtSpi>, window_node: NodeRef) -> Self {
        info!(target: "mimicwx::chat", "创建 ChatWnd: {who}");
        Self {
            who,
            atspi,
            window_node,
            edit_box_node: None,
            msg_list_node: None,
        }
    }

    pub async fn is_alive(&self) -> bool {
        if let Some(bbox) = self.atspi.bbox(&self.window_node).await {
            bbox.w > 0 && bbox.h > 0
        } else {
            false
        }
    }

    pub async fn init_edit_box(&mut self) {
        if self.edit_box_node.is_some() {
            return;
        }
        let win = self.window_node.clone();
        if let Some(node) = search::find_dfs(
            self.atspi.as_ref(),
            &win,
            &|role, _| {
                if role == "entry" || role == "text" {
                    SearchAction::Found
                } else if role == "list" {
                    SearchAction::Skip
                } else {
                    SearchAction::Recurse
                }
            },
            0, 15, 30,
        ).await {
            info!(target: "mimicwx::chat", "缓存输入框: {}", self.who);
            self.edit_box_node = Some(node);
        } else {
            info!(target: "mimicwx::chat", "未找到输入框, 使用偏移: {}", self.who);
        }
    }

    pub async fn init_msg_list(&mut self) {
        if self.msg_list_node.is_some() {
            return;
        }
        let win = self.window_node.clone();
        if let Some(node) = search::find_dfs(
            self.atspi.as_ref(),
            &win,
            &|role, name| {
                if match_message_list(role, name) {
                    SearchAction::Found
                } else if role == "list" {
                    SearchAction::Skip
                } else {
                    SearchAction::Recurse
                }
            },
            0, 15, 30,
        ).await {
            info!(target: "mimicwx::chat", "缓存消息列表: {}", self.who);
            self.msg_list_node = Some(node);
        } else {
            info!(target: "mimicwx::chat", "未找到消息列表: {}", self.who);
        }
    }

    pub async fn find_message_list(&self) -> Option<NodeRef> {
        search::find_bfs(self.atspi.as_ref(), &self.window_node, match_message_list).await
    }

    pub async fn send_message(
        &mut self,
        engine: &mut dyn InputDevice,
        text: &str,
        skip_verify: bool,
    ) -> Result<(bool, bool, String)> {
        let preview = text.lines().next().unwrap_or(text);
        info!(target: "mimicwx::send", "[ChatWnd] [{}] {preview}", self.who);

        self.activate_and_focus_input(engine).await?;

        engine.paste_text(text).await?;
        tokio::time::sleep(ms(300)).await;

        engine.press_enter().await?;
        tokio::time::sleep(ms(500)).await;

        let verified = if skip_verify {
            debug!(target: "mimicwx::send", "[ChatWnd] 跳过 UI 验证: [{}]", self.who);
            false
        } else {
            self.verify_sent(text).await
        };

        let msg = if verified { "消息已发送 (UI验证通过)" }
            else if skip_verify { "消息已发送 (DB验证模式)" }
            else { "消息已发送 (未验证)" };
        info!(target: "mimicwx::send", "[ChatWnd] 完成: [{}] {msg}", self.who);
        Ok((true, verified, msg.into()))
    }

    pub async fn send_image(
        &mut self,
        engine: &mut dyn InputDevice,
        image_path: &str,
    ) -> Result<(bool, bool, String)> {
        info!(target: "mimicwx::send", "[ChatWnd] 图片: [{}] {image_path}", self.who);

        self.activate_and_focus_input(engine).await?;

        engine.paste_image(image_path).await?;
        tokio::time::sleep(ms(500)).await;

        engine.press_enter().await?;
        tokio::time::sleep(ms(500)).await;

        info!(target: "mimicwx::send", "[ChatWnd] 图片完成: [{}]", self.who);
        Ok((true, false, "图片已发送 (独立窗口)".into()))
    }

    pub async fn activate_and_focus_input(
        &mut self,
        engine: &mut dyn InputDevice,
    ) -> Result<()> {
        let activated = engine.activate_window_by_title(&self.who, false).unwrap_or(false);
        if !activated {
            if let Some(bbox) = self.atspi.bbox(&self.window_node).await {
                let cx = bbox.x + bbox.w / 2;
                engine.click(cx, bbox.y + 30).await?;
            }
        }
        tokio::time::sleep(ms(300)).await;

        let edit_valid = if let Some(ref edit_node) = self.edit_box_node {
            self.atspi.bbox(edit_node).await
                .map(|b| b.w > 0 && b.h > 0)
                .unwrap_or(false)
        } else {
            false
        };

        if !edit_valid {
            if self.edit_box_node.is_some() {
                debug!(target: "mimicwx::chat", "输入框缓存失效, 重新搜索: {}", self.who);
            }
            self.edit_box_node = None;
            self.init_edit_box().await;
        }

        if let Some(ref edit_node) = self.edit_box_node {
            if let Some(eb) = self.atspi.bbox(edit_node).await {
                let (cx, cy) = eb.center();
                engine.click(cx, cy).await?;
                tokio::time::sleep(ms(200)).await;
            }
        } else {
            if let Some(bbox) = self.atspi.bbox(&self.window_node).await {
                let cx = bbox.x + bbox.w / 2;
                engine.click(cx, bbox.y + bbox.h - 50).await?;
                tokio::time::sleep(ms(200)).await;
            }
        }

        Ok(())
    }

    async fn verify_sent(&mut self, text: &str) -> bool {
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(ms(500)).await;
            }

            let cached_valid = if let Some(ref cached) = self.msg_list_node {
                self.atspi.bbox(cached).await
                    .map(|b| b.w > 0 && b.h > 0)
                    .unwrap_or(false)
            } else {
                false
            };

            if !cached_valid {
                if self.msg_list_node.is_some() {
                    debug!(target: "mimicwx::chat", "消息列表缓存失效, 重新搜索: {}", self.who);
                }
                self.msg_list_node = None;
                self.init_msg_list().await;
            }

            let msg_list = if let Some(ref cached) = self.msg_list_node {
                cached.clone()
            } else {
                match self.find_message_list().await {
                    Some(l) => l,
                    None => continue,
                }
            };
            if helpers::verify_sent_in_list(
                self.atspi.as_ref(),
                &msg_list,
                text,
                attempt,
            ).await {
                return true;
            }
        }
        false
    }
}
