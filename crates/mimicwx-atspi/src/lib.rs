//! mimicwx-atspi: AT-SPI2 底层原语
//!
//! 封装 zbus D-Bus 调用，提供节点遍历、属性读取、坐标获取等能力。
//! 所有 D-Bus 调用带 500ms 超时保护。
//!
//! # 模块结构
//! - [`connection`]: [`AtSpi`] 结构体 + 连接/重连策略
//! - [`node`]: [`NodeRef`] 节点引用 + [`registry()`] 根节点
//! - [`traits`]: [`AtspiClient`] trait (可 mock)
//! - [`search`]: BFS/DFS 搜索原语 (泛型, 兼容 `&dyn AtspiClient`)
//! - [`dump`]: 调试树导出
//! - [`helpers`]: 发送验证 + 轮询等待工具

pub mod connection;
pub mod node;
pub mod r#traits;
pub mod search;
pub mod dump;
pub mod helpers;

// 便捷 re-export
pub use connection::AtSpi;
pub use node::{NodeRef, registry};
pub use r#traits::AtspiClient;
pub use dump::dump_tree;
pub use mimicwx_core::{BBox, SearchAction, TreeNode};

// =====================================================================
// 为 AtSpi 实现 AtspiClient trait
// =====================================================================

use async_trait::async_trait;

#[async_trait]
impl AtspiClient for AtSpi {
    async fn child_count(&self, node: &NodeRef) -> i32 {
        AtSpi::child_count(self, node).await
    }

    async fn child_at(&self, node: &NodeRef, idx: i32) -> Option<NodeRef> {
        AtSpi::child_at(self, node, idx).await
    }

    async fn name(&self, node: &NodeRef) -> String {
        AtSpi::name(self, node).await
    }

    async fn role(&self, node: &NodeRef) -> String {
        AtSpi::role(self, node).await
    }

    async fn role_and_name(&self, node: &NodeRef) -> (String, String) {
        AtSpi::role_and_name(self, node).await
    }

    async fn bbox(&self, node: &NodeRef) -> Option<BBox> {
        AtSpi::bbox(self, node).await
    }

    async fn text(&self, node: &NodeRef) -> Option<String> {
        AtSpi::text(self, node).await
    }

    async fn description(&self, node: &NodeRef) -> String {
        AtSpi::description(self, node).await
    }

    async fn parent(&self, node: &NodeRef) -> Option<NodeRef> {
        AtSpi::parent(self, node).await
    }

    async fn get_states(&self, node: &NodeRef) -> u64 {
        AtSpi::get_states(self, node).await
    }

    async fn is_selected(&self, node: &NodeRef) -> bool {
        AtSpi::is_selected(self, node).await
    }

    async fn grab_focus(&self, node: &NodeRef) -> bool {
        AtSpi::grab_focus(self, node).await
    }
}
