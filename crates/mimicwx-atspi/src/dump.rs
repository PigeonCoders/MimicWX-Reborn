//! 调试树导出
//!
//! 导出 AT-SPI2 控件树结构, 限制 200 节点, 用于 API `/debug/tree` 端点。

use mimicwx_core::TreeNode;

use crate::node::NodeRef;
use crate::traits::AtspiClient;

const MAX_DUMP_NODES: u32 = 200;
const MAX_CHILDREN_PER_NODE: i32 = 20;

/// 导出 AT-SPI2 树（调试用，限制 200 节点）
pub async fn dump_tree<C: AtspiClient + ?Sized>(
    client: &C, root: &NodeRef, max_depth: u32,
) -> Vec<TreeNode> {
    let mut nodes = Vec::new();
    let mut count = 0u32;
    dump_dfs(client, root, 0, max_depth, &mut nodes, &mut count).await;
    nodes
}

fn dump_dfs<'a, C: AtspiClient + ?Sized>(
    client: &'a C, node: &'a NodeRef, depth: u32, max_depth: u32,
    out: &'a mut Vec<TreeNode>, count: &'a mut u32,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        if depth > max_depth || *count >= MAX_DUMP_NODES { return; }
        *count += 1;

        let (role, name, children) = tokio::join!(
            client.role(node),
            client.name(node),
            client.child_count(node)
        );

        out.push(TreeNode { depth, role: role.clone(), name: name.clone(), children });

        // 消息列表不递归
        if role == "list" && (name.contains("消息") || name.contains("Messages")) {
            return;
        }

        for i in 0..children.min(MAX_CHILDREN_PER_NODE) {
            if *count >= MAX_DUMP_NODES { return; }
            if let Some(child) = client.child_at(node, i).await {
                dump_dfs(client, &child, depth + 1, max_depth, out, count).await;
            }
        }
    })
}
