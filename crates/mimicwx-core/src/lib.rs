//! mimicwx-core: 基础层
//!
//! 提供所有 crate 共享的基础设施:
//! - [`error`]: 统一错误类型 [`MimicError`] + [`Result`] 别名
//! - [`config`]: 配置文件管理 [`AppConfig`] / [`load_config`] / [`save_listen_list`]
//! - [`types`]: 共享类型 [`BBox`] / [`SearchAction`] / [`TreeNode`]
//! - [`timing`]: 时间工具 [`ms()`]
//! - [`predicates`]: 消息列表谓词 [`is_message_list`] / [`match_message_list`]

pub mod error;
pub mod config;
pub mod types;
pub mod timing;
pub mod predicates;

pub use error::{MimicError, Result};
pub use config::{AppConfig, ApiConfig, ListenConfig, TimingConfig, load_config, save_listen_list};
pub use types::{BBox, SearchAction, TreeNode};
pub use timing::ms;
pub use predicates::{is_message_list, match_message_list, is_structural_role};
