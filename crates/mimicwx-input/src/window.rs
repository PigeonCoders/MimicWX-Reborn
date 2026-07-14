//! 窗口管理 — X11 原生 (替代 xdotool)
//!
//! 基于 EWMH 协议: `_NET_CLIENT_LIST` + `_NET_WM_NAME` 搜索窗口,
//! `_NET_ACTIVE_WINDOW` / `_NET_CLOSE_WINDOW` 激活/关闭窗口。

use anyhow::Result;
use tracing::{debug, info};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{self, AtomEnum, ClientMessageEvent, EventMask, ConnectionExt as _};

use crate::keyboard::InputEngine;

impl InputEngine {
    /// 按标题搜索窗口 (EWMH _NET_CLIENT_LIST + 标题匹配)
    ///
    /// `exact=true`: 精确匹配; `exact=false`: contains 匹配
    /// 返回匹配的 (window_id, window_name) 列表
    pub fn find_windows_by_title(&self, title: &str, exact: bool) -> Result<Vec<(u32, String)>> {
        let wm_name_atom = self.atom_net_wm_name;
        let utf8_atom = self.atom_utf8_string;
        let client_list_atom = self.atom_net_client_list;

        let windows: Vec<u32> = if let Ok(reply) = self.conn.get_property(
            false, self.screen_root, client_list_atom,
            u32::from(AtomEnum::WINDOW), 0, 4096,
        )?.reply() {
            if reply.format == 32 && !reply.value.is_empty() {
                reply.value.chunks_exact(4)
                    .map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect()
            } else {
                self.conn.query_tree(self.screen_root)?.reply()?.children
            }
        } else {
            self.conn.query_tree(self.screen_root)?.reply()?.children
        };

        let mut found = Vec::new();

        for &win in &windows {
            let name = if let Ok(reply) = self.conn.get_property(
                false, win, wm_name_atom, utf8_atom, 0, 1024,
            )?.reply() {
                if reply.value.is_empty() {
                    if let Ok(reply2) = self.conn.get_property(
                        false, win, u32::from(AtomEnum::WM_NAME), u32::from(AtomEnum::STRING), 0, 1024,
                    )?.reply() {
                        String::from_utf8_lossy(&reply2.value).to_string()
                    } else {
                        continue;
                    }
                } else {
                    String::from_utf8_lossy(&reply.value).to_string()
                }
            } else {
                continue;
            };

            let matched = if exact { name == title } else { name.contains(title) };
            if matched {
                found.push((win, name));
            }
        }
        Ok(found)
    }

    /// 通过窗口标题激活指定窗口 (X11 _NET_ACTIVE_WINDOW)
    ///
    /// 返回是否成功找到并激活了窗口
    pub fn activate_window_by_title(&self, title: &str, exact: bool) -> Result<bool> {
        let windows = self.find_windows_by_title(title, exact)?;
        if let Some((win, name)) = windows.first() {
            debug!(target: "mimicwx::input", "激活窗口: '{name}' (wid={win})");
            let active_atom = self.atom_net_active_window;
            let event = ClientMessageEvent {
                response_type: xproto::CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window: *win,
                type_: active_atom,
                data: [1u32, 0, 0, 0, 0].into(),
            };
            self.conn.send_event(
                false,
                self.screen_root,
                EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
                event,
            )?;
            self.conn.flush()?;
            Ok(true)
        } else {
            debug!(target: "mimicwx::input", "未找到标题匹配 '{title}' 的窗口");
            Ok(false)
        }
    }

    /// 通过窗口标题关闭指定窗口 (X11 _NET_CLOSE_WINDOW)
    pub fn close_window_by_title(&self, title: &str) -> Result<bool> {
        let windows = self.find_windows_by_title(title, false)?;
        if let Some((win, name)) = windows.first() {
            info!(target: "mimicwx::input", "关闭窗口: '{name}' (匹配 '{title}')");
            let close_atom = self.atom_net_close_window;
            let event = ClientMessageEvent {
                response_type: xproto::CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window: *win,
                type_: close_atom,
                data: [0u32; 5].into(),
            };
            self.conn.send_event(
                false,
                self.screen_root,
                EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
                event,
            )?;
            self.conn.flush()?;
            Ok(true)
        } else {
            debug!(target: "mimicwx::input", "未找到包含 '{title}' 的窗口");
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_match_exact_vs_contains() {
        let title = "微信 - MimicWX";
        assert!(title == "微信 - MimicWX"); // exact
        assert!(title.contains("微信"));    // contains
        assert!(!title.contains("QQ"));     // no match
    }

    #[test]
    fn test_window_name_candidates() {
        let candidates = vec![
            "微信",
            "微信 - 联系人",
            "WeChat",
            "ChatWnd - 朋友A",
        ];
        let matches: Vec<&str> = candidates.iter()
            .filter(|c| c.contains("微信"))
            .copied()
            .collect();
        assert_eq!(matches.len(), 2);
    }
}
