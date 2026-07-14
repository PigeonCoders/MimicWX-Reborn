//! InputDevice trait — X11 输入抽象接口
//!
//! 定义输入设备的标准操作接口, 具体实现为 [`crate::keyboard::InputEngine`]。
//! 用于 wechat 模块的依赖注入和 mock 测试。

use async_trait::async_trait;
use anyhow::Result;

/// X11 XTEST 输入设备 trait
///
/// 所有异步方法通过 `async-trait` 提供对象安全 (`dyn InputDevice + Send`)。
/// 同步的窗口管理方法直接返回 `Result`。
#[async_trait]
pub trait InputDevice: Send + Sync + 'static {
    // =================================================================
    // 键盘操作
    // =================================================================

    /// 模拟单次按键
    async fn press_key(&mut self, key_name: &str) -> Result<()>;

    /// 组合键 (如 "ctrl+f", "ctrl+v", "ctrl+a")
    async fn key_combo(&mut self, combo: &str) -> Result<()>;

    /// 逐字输入 ASCII 文本 (中文请用 paste_text)
    async fn type_text(&mut self, text: &str) -> Result<()>;

    /// 通过剪贴板粘贴文本 (支持中文)
    async fn paste_text(&mut self, text: &str) -> Result<()>;

    /// 通过剪贴板粘贴图片文件
    async fn paste_image(&mut self, image_path: &str) -> Result<()>;

    /// 发送 Enter 键
    async fn press_enter(&mut self) -> Result<()>;

    // =================================================================
    // 鼠标操作
    // =================================================================

    /// 鼠标移动到绝对坐标
    async fn move_mouse(&mut self, x: i32, y: i32) -> Result<()>;

    /// 鼠标单击
    async fn click(&mut self, x: i32, y: i32) -> Result<()>;

    /// 鼠标双击
    async fn double_click(&mut self, x: i32, y: i32) -> Result<()>;

    /// 鼠标右键点击
    async fn right_click(&mut self, x: i32, y: i32) -> Result<()>;

    /// 鼠标滚轮 (正=上, 负=下)
    async fn scroll(&mut self, x: i32, y: i32, clicks: i32) -> Result<()>;

    // =================================================================
    // 窗口管理
    // =================================================================

    /// 按标题搜索窗口
    fn find_windows_by_title(&self, title: &str, exact: bool) -> Result<Vec<(u32, String)>>;

    /// 通过窗口标题激活指定窗口
    fn activate_window_by_title(&self, title: &str, exact: bool) -> Result<bool>;

    /// 通过窗口标题关闭指定窗口
    fn close_window_by_title(&self, title: &str) -> Result<bool>;
}
