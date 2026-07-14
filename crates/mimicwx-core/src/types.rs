//! 共享类型
//!
//! 从原 `atspi.rs` 提取的不依赖 zbus 的类型:
//! - [`BBox`]: 控件坐标 (屏幕像素)
//! - [`SearchAction`]: DFS 搜索动作
//! - [`TreeNode`]: 调试用树节点信息
//!
//! `NodeRef` 依赖 `zbus::zvariant::OwnedObjectPath`, 留在 `mimicwx-atspi` crate。

use serde::Serialize;

/// 控件坐标 (屏幕像素)
#[derive(Debug, Clone, Copy)]
pub struct BBox {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl BBox {
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }
}

/// DFS 搜索动作
pub enum SearchAction {
    /// 匹配成功, 返回此节点
    Found,
    /// 不匹配, 继续递归子节点
    Recurse,
    /// 不匹配, 跳过此子树
    Skip,
}

/// 调试用: 树节点信息
#[derive(Serialize)]
pub struct TreeNode {
    pub depth: u32,
    pub role: String,
    pub name: String,
    pub children: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bbox_center() {
        let bbox = BBox { x: 100, y: 200, w: 80, h: 60 };
        assert_eq!(bbox.center(), (140, 230));
    }

    #[test]
    fn test_bbox_center_zero_origin() {
        let bbox = BBox { x: 0, y: 0, w: 100, h: 50 };
        assert_eq!(bbox.center(), (50, 25));
    }

    #[test]
    fn test_bbox_center_odd_dimensions() {
        let bbox = BBox { x: 0, y: 0, w: 101, h: 51 };
        assert_eq!(bbox.center(), (50, 25));
    }
}
