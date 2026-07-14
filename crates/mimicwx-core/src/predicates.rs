//! AT-SPI2 搜索谓词
//!
//! 从原 `utils.rs` 提取的不依赖 `AtSpi`/`NodeRef` 的纯函数谓词。
//! `verify_sent_in_list` / `wait_for` / `wait_for_result` 依赖 `AtSpi` + `NodeRef`,
//! 留在 `mimicwx-atspi` crate 的 helpers 模块。

use crate::types::SearchAction;

/// 判断节点是否是消息列表 (role="list", name 包含 "消息" 或 "Messages")
///
/// 用于 DFS/BFS 搜索的标准谓词
pub fn is_message_list(role: &str, name: &str) -> SearchAction {
    if role == "list" && (name.contains("消息") || name.contains("Messages") || name.contains("Message")) {
        SearchAction::Found
    } else {
        SearchAction::Recurse
    }
}

/// 消息列表 BFS 匹配器 (用于 find_bfs 的 Fn(&str, &str) -> bool 签名)
pub fn match_message_list(role: &str, name: &str) -> bool {
    role == "list" && (name.contains("消息") || name.contains("Messages") || name.contains("Message"))
}

/// 结构性角色: BFS 搜索时应当穿透的容器节点
pub fn is_structural_role(role: &str) -> bool {
    matches!(role,
        "filler" | "layered pane" | "panel" | "frame"
        | "scroll pane" | "viewport" | "section"
        | "split pane" | "splitter" | "page tab list"
        | "page tab" | "tool bar" | "" | "invalid"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_message_list_chinese() {
        assert!(matches!(is_message_list("list", "消息"), SearchAction::Found));
        assert!(matches!(is_message_list("list", "聊天消息"), SearchAction::Found));
    }

    #[test]
    fn test_is_message_list_english() {
        assert!(matches!(is_message_list("list", "Messages"), SearchAction::Found));
        assert!(matches!(is_message_list("list", "Message"), SearchAction::Found));
    }

    #[test]
    fn test_is_message_list_not_list_role() {
        assert!(matches!(is_message_list("panel", "消息"), SearchAction::Recurse));
        assert!(matches!(is_message_list("text", "Messages"), SearchAction::Recurse));
    }

    #[test]
    fn test_is_message_list_unrelated_name() {
        assert!(matches!(is_message_list("list", "联系人"), SearchAction::Recurse));
        assert!(matches!(is_message_list("list", ""), SearchAction::Recurse));
    }

    #[test]
    fn test_match_message_list() {
        assert!(match_message_list("list", "消息"));
        assert!(match_message_list("list", "Messages"));
        assert!(!match_message_list("list", "联系人"));
        assert!(!match_message_list("panel", "消息"));
    }

    #[test]
    fn test_is_structural_role() {
        assert!(is_structural_role("filler"));
        assert!(is_structural_role("panel"));
        assert!(is_structural_role("scroll pane"));
        assert!(is_structural_role(""));
        assert!(is_structural_role("invalid"));
        // 非结构性角色
        assert!(!is_structural_role("list"));
        assert!(!is_structural_role("text"));
        assert!(!is_structural_role("push button"));
        assert!(!is_structural_role("entry"));
    }
}
