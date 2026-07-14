//! 辅助工具函数
//!
//! 消除 wechat → chatwnd 反向依赖的公共函数, 从原 `utils.rs` 迁移。
//! 所有函数接受 [`AtspiClient`] trait, 可用于 mock 测试。

use std::sync::Arc;

use tracing::debug;

use crate::node::NodeRef;
use crate::traits::AtspiClient;

// =====================================================================
// 发送验证
// =====================================================================

/// 公共发送验证: 检查消息列表末尾是否包含指定文本
///
/// 被 `WeChat::verify_sent` 和 `ChatWnd::verify_sent` 共用, 消除 copy-paste。
/// 检查末尾 3 条消息, contains 双向匹配 + 长度校验。
pub async fn verify_sent_in_list<C: AtspiClient + ?Sized>(
    atspi: &C, msg_list: &NodeRef, text: &str, attempt: i32,
) -> bool {
    let count = atspi.child_count(msg_list).await;
    if count <= 0 { return false; }

    let check_range = 3.min(count);
    for i in (count - check_range)..count {
        if let Some(child) = atspi.child_at(msg_list, i).await {
            let name = atspi.name(&child).await;
            let trimmed = name.trim();
            let len_ok = !trimmed.is_empty()
                && trimmed.len() <= text.len() * 2 + 10
                && text.len() <= trimmed.len() * 2 + 10;
            if len_ok && (trimmed.contains(text) || text.contains(trimmed)) {
                debug!(target: "mimicwx::verify", "验证成功 (attempt {attempt})");
                return true;
            }
        }
    }
    false
}

// =====================================================================
// 轮询等待工具
// =====================================================================

/// 轮询等待条件满足 (布尔版)
///
/// 最多等待 `max_ms` 毫秒, 每 `interval_ms` 检查一次。
/// 返回: 条件是否在超时前满足。
pub async fn wait_for<C, F, Fut>(
    atspi: &Arc<C>, app: &NodeRef,
    max_ms: u64, interval_ms: u64,
    check: F,
) -> bool
where
    C: AtspiClient + ?Sized,
    F: Fn(&Arc<C>, &NodeRef) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(max_ms);
    while tokio::time::Instant::now() < deadline {
        if check(atspi, app).await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
    }
    false
}

/// 轮询等待并返回结果 (泛型版)
///
/// 最多等待 `max_ms` 毫秒, 每 `interval_ms` 检查一次。
/// 返回: 检查函数的结果 (`Some` = 成功, `None` = 超时)。
pub async fn wait_for_result<C, F, Fut, T>(
    atspi: &Arc<C>, app: &NodeRef,
    max_ms: u64, interval_ms: u64,
    check: F,
) -> Option<T>
where
    C: AtspiClient + ?Sized,
    F: Fn(&Arc<C>, &NodeRef) -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(max_ms);
    while tokio::time::Instant::now() < deadline {
        if let Some(result) = check(atspi, app).await {
            return Some(result);
        }
        tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
    }
    None
}
