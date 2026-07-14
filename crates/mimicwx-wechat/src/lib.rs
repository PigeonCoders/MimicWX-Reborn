//! mimicwx-wechat: 微信业务逻辑
//!
//! 依赖 mimicwx-atspi + mimicwx-input，提供:
//! - 微信应用/控件查找 (含缓存)
//! - 会话管理: 列表、切换 (ChatWith)
//! - 发送消息: 定位输入框 → 聚焦 → 粘贴验证 → 发送验证
//! - 独立窗口管理: ChatWnd 弹出/监听/关闭
//!
//! # 模块结构
//! - [`types`]: WeChatStatus / SessionInfo / CachedNode + 辅助函数
//! - [`chatwnd`]: ChatWnd 独立聊天窗口
//! - [`manager`]: WeChat 主结构体
//! - [`status`]: 状态检测 + 应用查找
//! - [`control`]: 控件查找 + 会话列表
//! - [`session`]: 会话切换 (ChatWith)
//! - [`listen`]: 独立窗口监听管理
//! - [`send`]: 消息/图片发送

pub mod types;
pub mod chatwnd;
pub mod manager;
pub mod status;
pub mod control;
pub mod session;
pub mod listen;
pub mod send;

pub use manager::WeChat;
pub use chatwnd::ChatWnd;
pub use types::{WeChatStatus, SessionInfo};
