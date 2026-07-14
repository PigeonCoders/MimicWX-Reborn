//! 统一错误类型
//!
//! 库 crate 返回 [`MimicError`], 应用 crate 顶层用 `anyhow` 统一处理。

use thiserror::Error;

/// MimicWX 统一错误类型
#[derive(Debug, Error)]
pub enum MimicError {
    #[error("AT-SPI2 error: {0}")]
    Atspi(String),

    #[error("X11 error: {0}")]
    X11(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Database key error: {0}")]
    DbKey(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("WeChat control not found: {0}")]
    ControlNotFound(String),

    #[error("WeChat not ready: {0}")]
    NotReady(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

impl MimicError {
    pub fn atspi(msg: impl Into<String>) -> Self {
        Self::Atspi(msg.into())
    }

    pub fn x11(msg: impl Into<String>) -> Self {
        Self::X11(msg.into())
    }

    pub fn database(msg: impl Into<String>) -> Self {
        Self::Database(msg.into())
    }

    pub fn db_key(msg: impl Into<String>) -> Self {
        Self::DbKey(msg.into())
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    pub fn control_not_found(msg: impl Into<String>) -> Self {
        Self::ControlNotFound(msg.into())
    }

    pub fn not_ready(msg: impl Into<String>) -> Self {
        Self::NotReady(msg.into())
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}

/// MimicWX 统一 Result 别名
pub type Result<T> = std::result::Result<T, MimicError>;
