//! 通用搜索原语 (BFS / DFS)
//!
//! 这些函数接受任何实现了 [`AtspiClient`] trait 的类型, including `&dyn AtspiClient`。
//! BFS 搜索使用结构性角色穿透; DFS 搜索通过 [`SearchAction`] 控制递归策略。

use mimicwx_core::{SearchAction, is_structural_role};

use crate::node::NodeRef;
use crate::traits::AtspiClient;

// 树遍历限制
const MAX_TREE_DEPTH: u32 = 20;
const MAX_CHILDREN_PER_NODE: i32 = 20;
const MAX_BFS_NODES: usize = 500;

/// BFS 查找节点 (结构性角色穿透, 最大深度 20, 每层最多 20 子节点)
///
/// `matcher(role, name) -> bool`: 返回 true 表示匹配
pub async fn find_bfs<C, F>(
    client: &C, root: &NodeRef,
    matcher: F,
) -> Option<NodeRef>
where
    C: AtspiClient + ?Sized,
    F: Fn(&str, &str) -> bool,
{
    find_bfs_limited(client, root, &matcher, MAX_BFS_NODES).await
}

/// BFS 查找节点 — 带节点数量上限
pub async fn find_bfs_limited<C, F>(
    client: &C, root: &NodeRef,
    matcher: &F,
    max_nodes: usize,
) -> Option<NodeRef>
where
    C: AtspiClient + ?Sized,
    F: Fn(&str, &str) -> bool,
{
    let mut frontier = vec![root.clone()];
    let mut visited = 0usize;

    for _depth in 0..MAX_TREE_DEPTH {
        if frontier.is_empty() { return None; }
        let mut next = Vec::new();

        for node in &frontier {
            let count = client.child_count(node).await;
            for i in 0..count.min(MAX_CHILDREN_PER_NODE) {
                visited += 1;
                if visited > max_nodes { return None; }
                if let Some(child) = client.child_at(node, i).await {
                    let (role, name) = client.role_and_name(&child).await;
                    if matcher(&role, &name) {
                        return Some(child);
                    }
                    if is_structural_role(&role) {
                        next.push(child);
                    }
                }
            }
        }
        frontier = next;
    }
    None
}

/// DFS 查找节点 (递归, 可控制跳过/递归/匹配)
///
/// `matcher(role, name) -> SearchAction`:
/// - [`SearchAction::Found`] = 匹配, 返回此节点
/// - [`SearchAction::Recurse`] = 不匹配, 但继续递归子节点
/// - [`SearchAction::Skip`] = 不匹配, 跳过此子树
#[allow(clippy::boxed_local)]
pub fn find_dfs<'a, C: AtspiClient + ?Sized>(
    client: &'a C, node: &'a NodeRef,
    matcher: &'a (dyn Fn(&str, &str) -> SearchAction + Send + Sync),
    depth: u32, max_depth: u32, max_children: i32,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<NodeRef>> + Send + 'a>>
{
    Box::pin(async move {
        if depth > max_depth { return None; }

        let count = client.child_count(node).await;
        for i in 0..count.min(max_children) {
            if let Some(child) = client.child_at(node, i).await {
                let (role, name) = client.role_and_name(&child).await;
                match matcher(&role, &name) {
                    SearchAction::Found => return Some(child),
                    SearchAction::Recurse => {
                        if let Some(found) = find_dfs(
                            client, &child, matcher, depth + 1, max_depth, max_children,
                        ).await {
                            return Some(found);
                        }
                    }
                    SearchAction::Skip => {}
                }
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(MAX_TREE_DEPTH, 20);
        assert_eq!(MAX_CHILDREN_PER_NODE, 20);
        assert_eq!(MAX_BFS_NODES, 500);
    }
}
