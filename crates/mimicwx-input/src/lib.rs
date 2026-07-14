//! X11 XTEST 输入引擎
//!
//! 通过 x11rb 使用 X11 XTEST 扩展注入键盘和鼠标事件。
//! 中文输入通过 X11 Selection（剪贴板）+ Ctrl+V 实现。图片通过 xclip + Ctrl+V。
//!
//! # 模块结构
//! - [`keyboard`]: 键盘操作 (press_key/key_combo/type_text/press_enter)
//! - [`clipboard`]: 剪贴板粘贴 (paste_text/paste_image)
//! - [`mouse`]: 鼠标操作 (move/click/double_click/right_click/scroll)
//! - [`window`]: 窗口管理 (find/activate/close)
//! - [`InputDevice`] trait: 抽象接口 (可 mock)

pub mod keyboard;
pub mod clipboard;
pub mod mouse;
pub mod window;
pub mod r#traits;

pub use r#traits::InputDevice;
pub use keyboard::InputEngine;

// =====================================================================
// 为 InputEngine 实现 InputDevice trait
// =====================================================================

use async_trait::async_trait;
use anyhow::Result;

#[async_trait]
impl InputDevice for InputEngine {
    async fn press_key(&mut self, key_name: &str) -> Result<()> {
        InputEngine::press_key(self, key_name).await
    }

    async fn key_combo(&mut self, combo: &str) -> Result<()> {
        InputEngine::key_combo(self, combo).await
    }

    async fn type_text(&mut self, text: &str) -> Result<()> {
        InputEngine::type_text(self, text).await
    }

    async fn paste_text(&mut self, text: &str) -> Result<()> {
        InputEngine::paste_text(self, text).await
    }

    async fn paste_image(&mut self, image_path: &str) -> Result<()> {
        InputEngine::paste_image(self, image_path).await
    }

    async fn move_mouse(&mut self, x: i32, y: i32) -> Result<()> {
        InputEngine::move_mouse(self, x, y).await
    }

    async fn click(&mut self, x: i32, y: i32) -> Result<()> {
        InputEngine::click(self, x, y).await
    }

    async fn double_click(&mut self, x: i32, y: i32) -> Result<()> {
        InputEngine::double_click(self, x, y).await
    }

    async fn right_click(&mut self, x: i32, y: i32) -> Result<()> {
        InputEngine::right_click(self, x, y).await
    }

    async fn scroll(&mut self, x: i32, y: i32, clicks: i32) -> Result<()> {
        InputEngine::scroll(self, x, y, clicks).await
    }

    async fn press_enter(&mut self) -> Result<()> {
        InputEngine::press_enter(self).await
    }

    fn find_windows_by_title(&self, title: &str, exact: bool) -> Result<Vec<(u32, String)>> {
        InputEngine::find_windows_by_title(self, title, exact)
    }

    fn activate_window_by_title(&self, title: &str, exact: bool) -> Result<bool> {
        InputEngine::activate_window_by_title(self, title, exact)
    }

    fn close_window_by_title(&self, title: &str) -> Result<bool> {
        InputEngine::close_window_by_title(self, title)
    }
}
