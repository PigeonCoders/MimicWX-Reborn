//! AtspiClient trait — AT-SPI2 抽象接口
//!
//! 定义 AT-SPI2 客户端的标准操作接口, 具体实现为 [`crate::AtSpi`]。
//! 用于 wechat/db 等模块的依赖注入和 mock 测试。

use async_trait::async_trait;
use mimicwx_core::BBox;

use crate::node::NodeRef;

/// AT-SPI2 客户端 trait
///
/// 所有方法均为 async, 通过 `async-trait` 提供对象安全 (`dyn AtspiClient`)。
/// 属性读取方法在 D-Bus 调用失败时返回默认值 (0/None/空串), 不返回 `Result`。
#[async_trait]
pub trait AtspiClient: Send + Sync {
    /// 子节点数量
    async fn child_count(&self, node: &NodeRef) -> i32;

    /// 获取第 idx 个子节点
    async fn child_at(&self, node: &NodeRef, idx: i32) -> Option<NodeRef>;

    /// 节点 Name 属性
    async fn name(&self, node: &NodeRef) -> String;

    /// 节点 RoleName
    async fn role(&self, node: &NodeRef) -> String;

    /// 并发获取 role + name (2 次 D-Bus 调用并行)
    async fn role_and_name(&self, node: &NodeRef) -> (String, String);

    /// 控件坐标 (屏幕像素)
    async fn bbox(&self, node: &NodeRef) -> Option<BBox>;

    /// 输入框文本内容
    async fn text(&self, node: &NodeRef) -> Option<String>;

    /// 节点 Description 属性
    async fn description(&self, node: &NodeRef) -> String;

    /// 父节点
    async fn parent(&self, node: &NodeRef) -> Option<NodeRef>;

    /// 64 位状态标志集合 (两个 u32 合并)
    async fn get_states(&self, node: &NodeRef) -> u64;

    /// 是否处于 SELECTED 状态 (bit 25)
    async fn is_selected(&self, node: &NodeRef) -> bool;

    /// 强制聚焦节点
    async fn grab_focus(&self, node: &NodeRef) -> bool;
}
