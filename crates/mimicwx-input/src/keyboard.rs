//! X11 XTEST 输入引擎 — 结构体 + 键盘操作
//!
//! [`InputEngine`] 持有 X11 连接和键盘映射, 其他模块 ([`crate::clipboard`],
//! [`crate::mouse`], [`crate::window`]) 通过 `impl InputEngine` 扩展。

use anyhow::{Context, Result};
use tracing::{debug, info};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{ConnectionExt as _, Keycode};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// X11 事件类型
pub(crate) const KEY_PRESS: u8 = 2;
pub(crate) const KEY_RELEASE: u8 = 3;
pub(crate) const BUTTON_PRESS: u8 = 4;
pub(crate) const BUTTON_RELEASE: u8 = 5;
pub(crate) const MOTION_NOTIFY: u8 = 6;

/// 延迟常量 (ms)
const KEY_HOLD_MS: u64 = 30;
const TYPING_DELAY_MS: u64 = 20;
pub(crate) const CLICK_HOLD_MS: u64 = 50;

/// X11 Keysym 常量
pub(crate) mod keysym {
    pub const XK_SPACE: u32 = 0x0020;
    pub const XK_RETURN: u32 = 0xFF0D;
    pub const XK_ESCAPE: u32 = 0xFF1B;
    pub const XK_TAB: u32 = 0xFF09;
    pub const XK_BACKSPACE: u32 = 0xFF08;
    pub const XK_DELETE: u32 = 0xFFFF;
    pub const XK_HOME: u32 = 0xFF50;
    pub const XK_END: u32 = 0xFF57;
    pub const XK_LEFT: u32 = 0xFF51;
    pub const XK_UP: u32 = 0xFF52;
    pub const XK_RIGHT: u32 = 0xFF53;
    pub const XK_DOWN: u32 = 0xFF54;
    pub const XK_SHIFT_L: u32 = 0xFFE1;
    pub const XK_CONTROL_L: u32 = 0xFFE3;
    pub const XK_ALT_L: u32 = 0xFFE4;
    pub const XK_F1: u32 = 0xFFBE;
    pub const XK_F2: u32 = 0xFFBF;
    pub const XK_F3: u32 = 0xFFC0;
    pub const XK_F4: u32 = 0xFFC1;
    pub const XK_F5: u32 = 0xFFC2;
}

/// X11 XTEST 输入引擎
///
/// 通过 x11rb 使用 X11 XTEST 扩展注入键盘和鼠标事件。
/// 缓存的 Atom 在 X11 Session 内永不变, 启动时一次性 intern。
pub struct InputEngine {
    pub(crate) conn: RustConnection,
    pub(crate) screen_root: u32,
    pub(crate) min_keycode: Keycode,
    pub(crate) max_keycode: Keycode,
    pub(crate) keysyms_per_keycode: u8,
    pub(crate) keysyms: Vec<u32>,
    pub(crate) atom_net_wm_name: u32,
    pub(crate) atom_utf8_string: u32,
    pub(crate) atom_net_client_list: u32,
    pub(crate) atom_net_active_window: u32,
    pub(crate) atom_net_close_window: u32,
    pub(crate) atom_clipboard: u32,
    pub(crate) atom_targets: u32,
}

impl InputEngine {
    /// 创建输入引擎
    pub fn new() -> Result<Self> {
        info!(target: "mimicwx::input", "初始化 X11 输入引擎...");

        let display_env = std::env::var("DISPLAY").unwrap_or_else(|_| ":1".into());
        let (conn, screen_num) = RustConnection::connect(Some(&display_env))
            .context(format!("连接 X11 失败 (DISPLAY={display_env})"))?;

        let screen = &conn.setup().roots[screen_num];
        let screen_root = screen.root;

        x11rb::protocol::xtest::get_version(&conn, 2, 2)
            .context("XTEST 扩展不可用")?
            .reply()
            .context("XTEST 版本查询失败")?;

        let setup = conn.setup();
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;
        let reply = conn.get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)?
            .reply()
            .context("获取键盘映射失败")?;

        let keysyms_per_keycode = reply.keysyms_per_keycode;
        let keysyms: Vec<u32> = reply.keysyms.iter().map(|k| (*k).into()).collect();

        let atom_net_wm_name = conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom;
        let atom_utf8_string = conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
        let atom_net_client_list = conn.intern_atom(false, b"_NET_CLIENT_LIST")?.reply()?.atom;
        let atom_net_active_window = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW")?.reply()?.atom;
        let atom_net_close_window = conn.intern_atom(false, b"_NET_CLOSE_WINDOW")?.reply()?.atom;
        let atom_clipboard = conn.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
        let atom_targets = conn.intern_atom(false, b"TARGETS")?.reply()?.atom;

        info!(target: "mimicwx::input", "X11 就绪 (DISPLAY={display_env}, keycodes={min_keycode}~{max_keycode})");

        Ok(Self {
            conn, screen_root, min_keycode, max_keycode, keysyms_per_keycode, keysyms,
            atom_net_wm_name, atom_utf8_string, atom_net_client_list,
            atom_net_active_window, atom_net_close_window,
            atom_clipboard, atom_targets,
        })
    }

    // =================================================================
    // Keysym 查找
    // =================================================================

    pub(crate) fn keysym_to_keycode(&self, keysym: u32) -> Option<(Keycode, bool)> {
        let per = self.keysyms_per_keycode as usize;
        let total = (self.max_keycode - self.min_keycode + 1) as usize;

        for i in 0..total {
            for j in 0..per {
                if self.keysyms[i * per + j] == keysym {
                    let keycode = self.min_keycode + i as u8;
                    let need_shift = j == 1;
                    return Some((keycode, need_shift));
                }
            }
        }
        None
    }

    fn char_to_keysym(ch: char) -> Option<u32> {
        match ch {
            ' ' => Some(keysym::XK_SPACE),
            '\n' => Some(keysym::XK_RETURN),
            '\t' => Some(keysym::XK_TAB),
            c if c.is_ascii() => Some(c as u32),
            _ => None,
        }
    }

    fn key_name_to_keysym(name: &str) -> Option<u32> {
        match name.to_lowercase().as_str() {
            "return" | "enter" => Some(keysym::XK_RETURN),
            "escape" | "esc" => Some(keysym::XK_ESCAPE),
            "tab" => Some(keysym::XK_TAB),
            "backspace" => Some(keysym::XK_BACKSPACE),
            "delete" => Some(keysym::XK_DELETE),
            "space" => Some(keysym::XK_SPACE),
            "home" => Some(keysym::XK_HOME),
            "end" => Some(keysym::XK_END),
            "left" => Some(keysym::XK_LEFT),
            "right" => Some(keysym::XK_RIGHT),
            "up" => Some(keysym::XK_UP),
            "down" => Some(keysym::XK_DOWN),
            "shift" => Some(keysym::XK_SHIFT_L),
            "ctrl" | "control" => Some(keysym::XK_CONTROL_L),
            "alt" => Some(keysym::XK_ALT_L),
            "f1" => Some(keysym::XK_F1),
            "f2" => Some(keysym::XK_F2),
            "f3" => Some(keysym::XK_F3),
            "f4" => Some(keysym::XK_F4),
            "f5" => Some(keysym::XK_F5),
            s if s.len() == 1 => Self::char_to_keysym(s.chars().next()?),
            _ => None,
        }
    }

    // =================================================================
    // 底层 XTEST 操作
    // =================================================================

    fn raw_key_press(&self, keycode: Keycode) -> Result<()> {
        self.conn.xtest_fake_input(KEY_PRESS, keycode, 0, self.screen_root, 0, 0, 0)?;
        self.conn.flush()?;
        Ok(())
    }

    fn raw_key_release(&self, keycode: Keycode) -> Result<()> {
        self.conn.xtest_fake_input(KEY_RELEASE, keycode, 0, self.screen_root, 0, 0, 0)?;
        self.conn.flush()?;
        Ok(())
    }

    // =================================================================
    // 键盘操作
    // =================================================================

    /// 模拟单次按键
    pub async fn press_key(&mut self, key_name: &str) -> Result<()> {
        let ks = Self::key_name_to_keysym(key_name)
            .ok_or_else(|| anyhow::anyhow!("未知按键: {key_name}"))?;
        let (keycode, need_shift) = self.keysym_to_keycode(ks)
            .ok_or_else(|| anyhow::anyhow!("按键无映射: {key_name}"))?;

        let shift_kc = if need_shift {
            self.keysym_to_keycode(keysym::XK_SHIFT_L).map(|(kc, _)| kc)
        } else { None };
        if let Some(skc) = shift_kc { self.raw_key_press(skc)?; }

        self.raw_key_press(keycode)?;
        tokio::time::sleep(std::time::Duration::from_millis(KEY_HOLD_MS)).await;
        self.raw_key_release(keycode)?;

        if let Some(skc) = shift_kc { self.raw_key_release(skc)?; }

        debug!(target: "mimicwx::input", "press_key: {key_name}");
        Ok(())
    }

    /// 组合键 (如 "ctrl+f", "ctrl+v", "ctrl+a")
    pub async fn key_combo(&mut self, combo: &str) -> Result<()> {
        let parts: Vec<&str> = combo.split('+').collect();
        let mut keycodes = Vec::new();

        for part in &parts {
            let ks = Self::key_name_to_keysym(part.trim())
                .ok_or_else(|| anyhow::anyhow!("未知按键: {part}"))?;
            let (kc, _) = self.keysym_to_keycode(ks)
                .ok_or_else(|| anyhow::anyhow!("按键无映射: {part}"))?;
            keycodes.push(kc);
        }

        for &kc in &keycodes {
            self.raw_key_press(kc)?;
            tokio::time::sleep(std::time::Duration::from_millis(KEY_HOLD_MS)).await;
        }
        for &kc in keycodes.iter().rev() {
            self.raw_key_release(kc)?;
        }

        debug!(target: "mimicwx::input", "key_combo: {combo}");
        Ok(())
    }

    /// 逐字输入 ASCII 文本 (中文请用 paste_text)
    pub async fn type_text(&mut self, text: &str) -> Result<()> {
        for ch in text.chars() {
            let ks = Self::char_to_keysym(ch)
                .ok_or_else(|| anyhow::anyhow!("字符无映射: '{ch}' — 请用 paste_text"))?;
            let (keycode, need_shift) = self.keysym_to_keycode(ks)
                .ok_or_else(|| anyhow::anyhow!("字符无 keycode: '{ch}'"))?;

            let shift_kc = if need_shift {
                self.keysym_to_keycode(keysym::XK_SHIFT_L).map(|(kc, _)| kc)
            } else { None };
            if let Some(skc) = shift_kc { self.raw_key_press(skc)?; }

            self.raw_key_press(keycode)?;
            tokio::time::sleep(std::time::Duration::from_millis(KEY_HOLD_MS)).await;
            self.raw_key_release(keycode)?;

            if let Some(skc) = shift_kc { self.raw_key_release(skc)?; }
            tokio::time::sleep(std::time::Duration::from_millis(TYPING_DELAY_MS)).await;
        }
        Ok(())
    }

    /// 发送 Enter 键
    pub async fn press_enter(&mut self) -> Result<()> {
        self.press_key("Return").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_to_keysym_ascii() {
        assert_eq!(InputEngine::char_to_keysym('a'), Some(0x61));
        assert_eq!(InputEngine::char_to_keysym('Z'), Some(0x5A));
        assert_eq!(InputEngine::char_to_keysym(' '), Some(keysym::XK_SPACE));
        assert_eq!(InputEngine::char_to_keysym('\n'), Some(keysym::XK_RETURN));
        assert_eq!(InputEngine::char_to_keysym('\t'), Some(keysym::XK_TAB));
    }

    #[test]
    fn test_char_to_keysym_non_ascii() {
        assert_eq!(InputEngine::char_to_keysym('你'), None);
        assert_eq!(InputEngine::char_to_keysym('€'), None);
    }

    #[test]
    fn test_key_name_to_keysym_named() {
        assert_eq!(InputEngine::key_name_to_keysym("return"), Some(keysym::XK_RETURN));
        assert_eq!(InputEngine::key_name_to_keysym("Enter"), Some(keysym::XK_RETURN));
        assert_eq!(InputEngine::key_name_to_keysym("ctrl"), Some(keysym::XK_CONTROL_L));
        assert_eq!(InputEngine::key_name_to_keysym("CONTROL"), Some(keysym::XK_CONTROL_L));
        assert_eq!(InputEngine::key_name_to_keysym("esc"), Some(keysym::XK_ESCAPE));
        assert_eq!(InputEngine::key_name_to_keysym("f1"), Some(keysym::XK_F1));
        assert_eq!(InputEngine::key_name_to_keysym("f5"), Some(keysym::XK_F5));
        assert_eq!(InputEngine::key_name_to_keysym("space"), Some(keysym::XK_SPACE));
    }

    #[test]
    fn test_key_name_to_keysym_single_char() {
        // key_name_to_keysym 会先 .to_lowercase(), 所以大写字母映射到小写 keysym
        assert_eq!(InputEngine::key_name_to_keysym("a"), Some(0x61));
        assert_eq!(InputEngine::key_name_to_keysym("Z"), Some(0x7A)); // → 'z' = 0x7A
    }

    #[test]
    fn test_key_name_to_keysym_unknown() {
        assert_eq!(InputEngine::key_name_to_keysym("unknown"), None);
        assert_eq!(InputEngine::key_name_to_keysym("f6"), None);
        assert_eq!(InputEngine::key_name_to_keysym(""), None);
    }

    #[test]
    fn test_keysym_constants_x11_standard() {
        assert_eq!(keysym::XK_SPACE, 0x0020);
        assert_eq!(keysym::XK_RETURN, 0xFF0D);
        assert_eq!(keysym::XK_ESCAPE, 0xFF1B);
        assert_eq!(keysym::XK_SHIFT_L, 0xFFE1);
        assert_eq!(keysym::XK_CONTROL_L, 0xFFE3);
    }
}
