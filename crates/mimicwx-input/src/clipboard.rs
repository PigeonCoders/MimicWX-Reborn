//! 剪贴板粘贴 — 文本 (X11 Selection 协议) + 图片 (xclip)
//!
//! 文本粘贴通过 X11 Selection 协议直接设置剪贴板, 无需 xclip 子进程。
//! 流程: spawn_blocking 中获取 CLIPBOARD ownership + 事件循环,
//!       main thread 发送 Ctrl+V, 触发 SelectionRequest。

use anyhow::{Context, Result};
use tracing::info;
use x11rb::connection::Connection as _;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::protocol::xproto::{PropMode, WindowClass, CreateWindowAux, SelectionNotifyEvent, AtomEnum, EventMask, ConnectionExt as _};
use x11rb::protocol::Event;

use crate::keyboard::InputEngine;

impl InputEngine {
    /// 通过剪贴板粘贴文本 (支持中文、空格等任意字符)
    pub async fn paste_text(&mut self, text: &str) -> Result<()> {
        self.clipboard_paste(text).await
    }

    async fn clipboard_paste(&mut self, text: &str) -> Result<()> {
        info!(target: "mimicwx::input", "粘贴文本: {}字符", text.len());

        let text_owned = text.to_string();
        let display_env = std::env::var("DISPLAY").unwrap_or_else(|_| ":1".into());

        let clipboard_atom = self.atom_clipboard;
        let utf8_atom = self.atom_utf8_string;
        let targets_atom_cached = self.atom_targets;

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();

        let handle = tokio::task::spawn_blocking(move || -> Result<()> {
            let (conn, screen_num) = x11rb::rust_connection::RustConnection::connect(Some(&display_env))
                .context("X11 clipboard 连接失败")?;
            let screen = &conn.setup().roots[screen_num];

            let clipboard = clipboard_atom;
            let utf8_string = utf8_atom;
            let targets_atom = targets_atom_cached;

            let win = conn.generate_id()?;
            conn.create_window(
                0, win, screen.root,
                0, 0, 1, 1, 0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new(),
            )?;
            conn.set_selection_owner(win, clipboard, x11rb::CURRENT_TIME)?;
            conn.flush()?;

            let owner = conn.get_selection_owner(clipboard)?.reply()?.owner;
            if owner != win {
                conn.destroy_window(win)?;
                conn.flush()?;
                anyhow::bail!("无法获取 CLIPBOARD ownership");
            }

            let _ = ready_tx.send(());

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);

            while std::time::Instant::now() < deadline {
                if let Ok(Some(event)) = conn.poll_for_event() {
                    match event {
                        Event::SelectionRequest(req) => {
                            let mut reply = SelectionNotifyEvent {
                                response_type: 31,
                                sequence: 0,
                                time: req.time,
                                requestor: req.requestor,
                                selection: req.selection,
                                target: req.target,
                                property: 0u32.into(),
                            };

                            if req.target == targets_atom {
                                let targets = [targets_atom, utf8_string, AtomEnum::STRING.into()];
                                let _ = conn.change_property32(
                                    PropMode::REPLACE, req.requestor, req.property,
                                    AtomEnum::ATOM, &targets,
                                );
                                reply.property = req.property;
                            } else if req.target == utf8_string || req.target == u32::from(AtomEnum::STRING) {
                                let _ = conn.change_property8(
                                    PropMode::REPLACE, req.requestor, req.property,
                                    utf8_string, text_owned.as_bytes(),
                                );
                                reply.property = req.property;
                            }

                            let _ = conn.send_event(false, req.requestor, EventMask::NO_EVENT, reply);
                            let _ = conn.flush();

                            if req.target == utf8_string || req.target == u32::from(AtomEnum::STRING) {
                                let extra_deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
                                while std::time::Instant::now() < extra_deadline {
                                    if let Ok(Some(Event::SelectionRequest(req2))) = conn.poll_for_event() {
                                        let mut r2 = SelectionNotifyEvent {
                                            response_type: 31, sequence: 0,
                                            time: req2.time, requestor: req2.requestor,
                                            selection: req2.selection, target: req2.target,
                                            property: 0u32.into(),
                                        };
                                        if req2.target == targets_atom {
                                            let targets = [targets_atom, utf8_string, AtomEnum::STRING.into()];
                                            let _ = conn.change_property32(PropMode::REPLACE, req2.requestor, req2.property, AtomEnum::ATOM, &targets);
                                            r2.property = req2.property;
                                        } else if req2.target == utf8_string || req2.target == u32::from(AtomEnum::STRING) {
                                            let _ = conn.change_property8(PropMode::REPLACE, req2.requestor, req2.property, utf8_string, text_owned.as_bytes());
                                            r2.property = req2.property;
                                        }
                                        let _ = conn.send_event(false, req2.requestor, EventMask::NO_EVENT, r2);
                                        let _ = conn.flush();
                                    } else {
                                        std::thread::sleep(std::time::Duration::from_millis(10));
                                    }
                                }
                                break;
                            }
                        }
                        Event::SelectionClear(_) => break,
                        _ => {}
                    }
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }

            conn.destroy_window(win)?;
            conn.flush()?;
            Ok(())
        });

        let _ = ready_rx.await;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        self.key_combo("ctrl+v").await?;

        handle.await
            .context("clipboard blocking task panicked")??;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        Ok(())
    }

    /// 通过剪贴板粘贴图片文件 (xclip + Ctrl+V)
    pub async fn paste_image(&mut self, image_path: &str) -> Result<()> {
        info!(target: "mimicwx::input", "粘贴图片: {image_path}");

        let mime = if image_path.ends_with(".png") {
            "image/png"
        } else if image_path.ends_with(".jpg") || image_path.ends_with(".jpeg") {
            "image/jpeg"
        } else if image_path.ends_with(".gif") {
            "image/gif"
        } else if image_path.ends_with(".bmp") {
            "image/bmp"
        } else {
            "image/png"
        };

        let status = tokio::process::Command::new("xclip")
            .args(["-selection", "clipboard", "-t", mime, "-i", image_path])
            .status()
            .await
            .context("启动 xclip 失败 (图片)")?;

        if !status.success() {
            anyhow::bail!("xclip 图片复制失败: exit={:?}", status.code());
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        self.key_combo("ctrl+v").await?;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_mime_detection() {
        assert_eq!(mime_for("file.png"), "image/png");
        assert_eq!(mime_for("file.jpg"), "image/jpeg");
        assert_eq!(mime_for("file.jpeg"), "image/jpeg");
        assert_eq!(mime_for("file.gif"), "image/gif");
        assert_eq!(mime_for("file.bmp"), "image/bmp");
        assert_eq!(mime_for("file.webp"), "image/png"); // 默认
        assert_eq!(mime_for("no_extension"), "image/png"); // 默认
    }

    fn mime_for(path: &str) -> &'static str {
        if path.ends_with(".png") {
            "image/png"
        } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
            "image/jpeg"
        } else if path.ends_with(".gif") {
            "image/gif"
        } else if path.ends_with(".bmp") {
            "image/bmp"
        } else {
            "image/png"
        }
    }
}
