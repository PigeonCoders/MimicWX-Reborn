//! mimicwx-db: SQLCipher 数据库监听
//!
//! 通过 SQLCipher 解密 + fanotify 监听 WAL 文件变化，实现:
//! - 联系人查询 (contact.db)
//! - 会话列表 (session.db)
//! - 增量消息获取 (message_N.db)
//! - 发送验证 (事件驱动)
//!
//! 模块拆分:
//! - [`types`]: 类型定义 (ContactInfo, MsgContent, DbMessage 等)
//! - [`key`]: 密钥管理 (FFI, 三级匹配)
//! - [`wcdb`]: WCDB 兼容 (Zstd 解压, 表结构发现)
//! - [`parser`]: 消息内容解析 (16+ 种消息类型)
//! - [`listener`]: WAL fanotify 监听
//! - [`contacts`]: 联系人/群成员 (impl DbManager)
//! - [`manager`]: DbManager 主结构 + 核心方法

mod contacts;
mod key;
mod listener;
mod manager;
mod parser;
mod types;
mod wcdb;

pub use manager::DbManager;
pub use types::{
    AppKind, ChatRecordItem, ContactInfo, DbMessage, DbSessionInfo, GroupMemberInfo, MsgContent,
    RawMsg, TableMeta,
};
