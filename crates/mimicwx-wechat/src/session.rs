//! 会话切换 (ChatWith)

use anyhow::Result;
use tracing::{debug, info};

use mimicwx_atspi::SearchAction;
use mimicwx_atspi::helpers::wait_for;
use mimicwx_core::{is_message_list, ms};
use mimicwx_input::InputDevice;

use crate::WeChat;

impl WeChat {
    pub async fn chat_with(
        &self,
        engine: &mut dyn InputDevice,
        who: &str,
    ) -> Result<Option<String>> {
        {
            let current = self.current_chat.lock().await;
            if let Some(ref name) = *current {
                if name == who {
                    debug!(target: "mimicwx::session", "已在聊天 [{who}], 跳过切换");
                    return Ok(Some(who.to_string()));
                }
            }
        }

        info!(target: "mimicwx::session", "ChatWith: {who}");

        self.focus_main_window(engine).await;

        let app = self.find_app().await
            .ok_or_else(|| anyhow::anyhow!("找不到微信应用"))?;

        if let Some(list) = self.find_session_list(&app).await {
            if let Some(item) = self.find_session(&list, who).await {
                if let Some(bbox) = self.atspi.bbox(&item).await {
                    let (cx, cy) = bbox.center();
                    debug!(target: "mimicwx::session", "会话列表找到 [{who}], 点击 ({cx}, {cy})");
                    engine.click(cx, cy).await?;
                    let loaded = wait_for(&self.atspi, &app, 1500, 50,
                        |atspi, app| {
                            let atspi = atspi.clone();
                            let app = app.clone();
                            async move {
                                mimicwx_atspi::search::find_dfs(
                                    atspi.as_ref(),
                                    &app,
                                    &is_message_list,
                                    0, 18, 20,
                                ).await.is_some()
                            }
                        }
                    ).await;
                    debug!(target: "mimicwx::session", "点击后消息列表: {}", if loaded { "已就绪" } else { "超时" });
                    *self.current_chat.lock().await = Some(who.to_string());
                    return Ok(Some(who.to_string()));
                }
            }
        }

        debug!(target: "mimicwx::session", "列表未找到 [{who}], 进入搜索");

        engine.key_combo("ctrl+f").await?;
        wait_for(&self.atspi, &app, 800, 50,
            |atspi, app| {
                let atspi = atspi.clone();
                let app = app.clone();
                async move {
                    mimicwx_atspi::search::find_dfs(
                        atspi.as_ref(),
                        &app,
                        &|role, _| {
                            if role == "entry" || role == "text" {
                                SearchAction::Found
                            } else { SearchAction::Recurse }
                        },
                        0, 18, 20,
                    ).await.is_some()
                }
            }
        ).await;

        engine.key_combo("ctrl+a").await?;
        tokio::time::sleep(ms(100)).await;

        engine.paste_text(who).await?;
        wait_for(&self.atspi, &app, 2000, 100,
            |atspi, app| {
                let atspi = atspi.clone();
                let app = app.clone();
                async move {
                    mimicwx_atspi::search::find_dfs(
                        atspi.as_ref(),
                        &app,
                        &|role, name| {
                            if role == "list" && !name.contains("Chats") && !name.contains("会话") && !name.is_empty() {
                                SearchAction::Found
                            } else { SearchAction::Recurse }
                        },
                        0, 18, 20,
                    ).await.is_some()
                }
            }
        ).await;

        engine.press_enter().await?;
        let loaded = wait_for(&self.atspi, &app, 2000, 50,
            |atspi, app| {
                let atspi = atspi.clone();
                let app = app.clone();
                async move {
                    mimicwx_atspi::search::find_dfs(
                        atspi.as_ref(),
                        &app,
                        &is_message_list,
                        0, 18, 20,
                    ).await.is_some()
                }
            }
        ).await;
        debug!(target: "mimicwx::session", "搜索切换后消息列表: {}", if loaded { "已就绪" } else { "超时" });

        engine.press_key("Escape").await?;
        wait_for(&self.atspi, &app, 800, 50,
            |atspi, app| {
                let atspi = atspi.clone();
                let app = app.clone();
                async move {
                    mimicwx_atspi::search::find_dfs(
                        atspi.as_ref(),
                        &app,
                        &is_message_list,
                        0, 18, 20,
                    ).await.is_some()
                }
            }
        ).await;

        if self.find_message_list(&app).await.is_some() {
            debug!(target: "mimicwx::session", "搜索切换成功: {who}");
            if !who.contains("@chatroom") {
                *self.current_chat.lock().await = Some(who.to_string());
            }
            Ok(Some(who.to_string()))
        } else {
            info!(target: "mimicwx::session", "搜索未找到: [{who}]");
            *self.current_chat.lock().await = None;
            Ok(None)
        }
    }
}
