//! 鼠标操作 — 移动、单击、双击、右键、滚轮

use anyhow::Result;
use tracing::debug;
use x11rb::connection::Connection as _;
use x11rb::protocol::xtest::ConnectionExt as _;

use crate::keyboard::{InputEngine, BUTTON_PRESS, BUTTON_RELEASE, MOTION_NOTIFY, CLICK_HOLD_MS};

impl InputEngine {
    /// 鼠标移动到绝对坐标
    pub async fn move_mouse(&mut self, x: i32, y: i32) -> Result<()> {
        self.conn.xtest_fake_input(MOTION_NOTIFY, 0, 0, self.screen_root, x as i16, y as i16, 0)?;
        self.conn.flush()?;
        debug!(target: "mimicwx::input", "move_mouse: ({x}, {y})");
        Ok(())
    }

    /// 鼠标单击
    pub async fn click(&mut self, x: i32, y: i32) -> Result<()> {
        self.move_mouse(x, y).await?;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        self.conn.xtest_fake_input(BUTTON_PRESS, 1, 0, self.screen_root, 0, 0, 0)?;
        self.conn.flush()?;
        tokio::time::sleep(std::time::Duration::from_millis(CLICK_HOLD_MS)).await;

        self.conn.xtest_fake_input(BUTTON_RELEASE, 1, 0, self.screen_root, 0, 0, 0)?;
        self.conn.flush()?;

        debug!(target: "mimicwx::input", "click: ({x}, {y})");
        Ok(())
    }

    /// 鼠标双击
    pub async fn double_click(&mut self, x: i32, y: i32) -> Result<()> {
        self.click(x, y).await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        self.click(x, y).await?;
        Ok(())
    }

    /// 鼠标右键点击
    #[allow(dead_code)]
    pub async fn right_click(&mut self, x: i32, y: i32) -> Result<()> {
        self.move_mouse(x, y).await?;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        self.conn.xtest_fake_input(BUTTON_PRESS, 3, 0, self.screen_root, 0, 0, 0)?;
        self.conn.flush()?;
        tokio::time::sleep(std::time::Duration::from_millis(CLICK_HOLD_MS)).await;

        self.conn.xtest_fake_input(BUTTON_RELEASE, 3, 0, self.screen_root, 0, 0, 0)?;
        self.conn.flush()?;

        debug!(target: "mimicwx::input", "right_click: ({x}, {y})");
        Ok(())
    }

    /// 鼠标滚轮 (正=上, 负=下)
    ///
    /// X11: button 4 = scroll up, button 5 = scroll down
    #[allow(dead_code)]
    pub async fn scroll(&mut self, x: i32, y: i32, clicks: i32) -> Result<()> {
        self.move_mouse(x, y).await?;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let button: u8 = if clicks > 0 { 4 } else { 5 };
        for _ in 0..clicks.unsigned_abs() {
            self.conn.xtest_fake_input(BUTTON_PRESS, button, 0, self.screen_root, 0, 0, 0)?;
            self.conn.xtest_fake_input(BUTTON_RELEASE, button, 0, self.screen_root, 0, 0, 0)?;
            self.conn.flush()?;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        debug!(target: "mimicwx::input", "scroll: ({x}, {y}) clicks={clicks}");
        Ok(())
    }
}
