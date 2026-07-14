//! 消息发送

use anyhow::Result;
use tracing::{debug, info};

use mimicwx_atspi::helpers::verify_sent_in_list;
use mimicwx_core::ms;
use mimicwx_input::InputDevice;

use crate::WeChat;

impl WeChat {
    pub async fn send_message(
        &self,
        engine: &mut dyn InputDevice,
        to: &str,
        text: &str,
        at: &[String],
        skip_verify: bool,
    ) -> Result<(bool, bool, String)> {
        let preview = text.lines().next().unwrap_or(text);
        info!(target: "mimicwx::send", "[{to}] {preview} (@ {} 人)", at.len());

        if self.check_listen_window(to).await {
            let mut windows = self.listen_windows.lock().await;
            if let Some(chatwnd) = windows.get_mut(to) {
                debug!(target: "mimicwx::send", "使用独立窗口: {to}");
                chatwnd.activate_and_focus_input(engine).await?;
                type_at_mentions(engine, at, self.get_at_delay_ms()).await?;
                return chatwnd.send_message(engine, text, skip_verify).await;
            }
        }

        if !self.prepare_main_send(engine, to, false).await? {
            return Ok((false, false, format!("未找到聊天: {to}")));
        }

        let app = self.find_app().await
            .ok_or_else(|| anyhow::anyhow!("找不到微信应用"))?;

        type_at_mentions(engine, at, self.get_at_delay_ms()).await?;

        engine.paste_text(text).await?;
        tokio::time::sleep(ms(300)).await;

        engine.press_enter().await?;
        tokio::time::sleep(ms(500)).await;

        let verified = if skip_verify {
            debug!(target: "mimicwx::send", "跳过 UI 验证 (由 DB 验证): [{to}]");
            false
        } else {
            self.verify_sent(&app, text).await
        };

        let msg = if verified { "消息已发送" } else { "消息已发送 (未验证)" };
        info!(target: "mimicwx::send", "完成: [{to}] verified={verified}");
        Ok((true, verified, msg.into()))
    }

    pub async fn send_image(
        &self,
        engine: &mut dyn InputDevice,
        to: &str,
        image_path: &str,
    ) -> Result<(bool, bool, String)> {
        info!(target: "mimicwx::send", "图片: [{to}] {image_path}");

        if self.check_listen_window(to).await {
            let mut windows = self.listen_windows.lock().await;
            if let Some(chatwnd) = windows.get_mut(to) {
                debug!(target: "mimicwx::send", "使用独立窗口: {to}");
                return chatwnd.send_image(engine, image_path).await;
            }
        }

        if !self.prepare_main_send(engine, to, true).await? {
            return Ok((false, false, format!("未找到聊天: {to}")));
        }

        engine.paste_image(image_path).await?;
        tokio::time::sleep(ms(500)).await;

        engine.press_enter().await?;

        info!(target: "mimicwx::send", "图片发送完成: [{to}]");
        Ok((true, false, "图片已发送".into()))
    }

    pub(crate) async fn prepare_main_send(
        &self,
        engine: &mut dyn InputDevice,
        to: &str,
        force_switch: bool,
    ) -> Result<bool> {
        if force_switch {
            *self.current_chat.lock().await = None;
        }
        let chat_result = self.chat_with(engine, to).await?;
        if chat_result.is_none() {
            return Ok(false);
        }
        tokio::time::sleep(ms(300)).await;
        Ok(true)
    }

    async fn verify_sent(&self, app: &mimicwx_atspi::NodeRef, text: &str) -> bool {
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(ms(500)).await;
            }
            if let Some(msg_list) = self.find_message_list(app).await {
                if verify_sent_in_list(self.atspi.as_ref(), &msg_list, text, attempt).await {
                    return true;
                }
            }
        }
        false
    }
}

async fn type_at_mentions(
    engine: &mut dyn InputDevice,
    at: &[String],
    delay_ms: u64,
) -> Result<()> {
    for name in at {
        if name.is_empty() { continue; }
        debug!(target: "mimicwx::send", "输入 @: {name}");
        engine.type_text("@").await?;
        tokio::time::sleep(ms(delay_ms)).await;
        engine.paste_text(name).await?;
        tokio::time::sleep(ms(delay_ms)).await;
        engine.press_enter().await?;
        tokio::time::sleep(ms(delay_ms * 2 / 3)).await;
    }
    Ok(())
}
