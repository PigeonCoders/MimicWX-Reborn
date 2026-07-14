//! 配置文件管理
//!
//! 配置文件搜索路径 (按优先级):
//! 1. `./config.toml`
//! 2. `/home/wechat/mimicwx-reborn/config.toml`
//! 3. `/etc/mimicwx/config.toml`

use serde::Deserialize;
use std::path::PathBuf;
use tracing::{info, warn};

// =====================================================================
// 配置结构体
// =====================================================================

#[derive(Debug, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub listen: ListenConfig,
    #[serde(default)]
    pub timing: TimingConfig,
}

#[derive(Debug, Deserialize, Default)]
pub struct ApiConfig {
    /// API 认证 Token (留空或不配置则不启用认证)
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListenConfig {
    /// 启动后自动弹出独立窗口并监听的对象
    #[serde(default)]
    pub auto: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct TimingConfig {
    /// @ 输入流程中每步的等待时间 (毫秒)
    #[serde(default = "default_at_delay")]
    pub at_delay_ms: u64,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self { at_delay_ms: 300 }
    }
}

fn default_at_delay() -> u64 { 300 }

// =====================================================================
// 配置加载与保存
// =====================================================================

/// 加载配置文件 (搜索多个路径)
/// 返回 (配置, 配置文件路径)
pub fn load_config() -> (AppConfig, Option<PathBuf>) {
    let search_paths = [
        PathBuf::from("./config.toml"),
        PathBuf::from("/home/wechat/mimicwx-reborn/config.toml"),
        PathBuf::from("/etc/mimicwx/config.toml"),
    ];
    for path in &search_paths {
        if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(content) => match toml::from_str::<AppConfig>(&content) {
                    Ok(config) => {
                        info!(target: "mimicwx::config", "配置已加载: {}", path.display());
                        return (config, Some(path.clone()));
                    }
                    Err(e) => {
                        warn!(target: "mimicwx::config", "配置解析失败: {} - {e}", path.display());
                    }
                },
                Err(e) => {
                    warn!(target: "mimicwx::config", "配置读取失败: {} - {e}", path.display());
                }
            }
        }
    }
    info!(target: "mimicwx::config", "未找到配置文件, 使用默认配置");
    (AppConfig::default(), None)
}

/// 保存监听列表到 config.toml (仅替换 auto = [...] 行, 保留注释和格式)
pub fn save_listen_list(config_path: &std::path::Path, listen_list: &[String]) {
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            warn!(target: "mimicwx::config", "无法读取配置: {e}");
            return;
        }
    };

    // 构造新的 auto 行 (横排格式, 与用户原始风格一致)
    let new_auto = if listen_list.is_empty() {
        "auto = []".to_string()
    } else {
        let items: Vec<_> = listen_list.iter().map(|s| format!("\"{}\"", s)).collect();
        format!("auto = [{}]", items.join(","))
    };

    // 逐行扫描, 找到非注释的 auto = [...] 行并替换
    // (跳过 # 开头的注释行, 避免误匹配 "# 示例: auto = [...]")
    let mut new_lines: Vec<String> = Vec::new();
    let mut found = false;
    let mut skip_continuation = false; // 跨行数组: 跳过后续行直到 ]
    for line in content.lines() {
        if skip_continuation {
            if line.contains(']') {
                skip_continuation = false;
            }
            continue; // 跳过跨行数组的中间行
        }
        let trimmed = line.trim();
        if !trimmed.starts_with('#') && trimmed.starts_with("auto") && trimmed.contains('=') {
            // 这是真正的 auto = [...] 行
            if trimmed.contains('[') && !trimmed.contains(']') {
                // 跨行数组: auto = [\n  "a",\n  "b",\n]
                skip_continuation = true;
            }
            new_lines.push(new_auto.clone());
            found = true;
        } else {
            new_lines.push(line.to_string());
        }
    }
    let new_content = if found {
        new_lines.join("\n")
    } else if content.contains("[listen]") {
        // 有 [listen] 段但无 auto 行, 在段后追加
        content.replace("[listen]", &format!("[listen]\n{}", new_auto))
    } else {
        // 无 [listen] 段, 在文件末尾追加
        format!("{content}\n[listen]\n{new_auto}\n")
    };

    match std::fs::write(config_path, new_content) {
        Ok(_) => info!(target: "mimicwx::config", "监听列表已保存到 {}", config_path.display()),
        Err(e) => warn!(target: "mimicwx::config", "保存配置失败: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.timing.at_delay_ms, 300);
        assert!(config.listen.auto.is_empty());
        assert!(config.api.token.is_none());
    }

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[api]
token = "secret123"

[listen]
auto = ["张三", "群聊1"]

[timing]
at_delay_ms = 500
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.api.token.as_deref(), Some("secret123"));
        assert_eq!(config.listen.auto, vec!["张三", "群聊1"]);
        assert_eq!(config.timing.at_delay_ms, 500);
    }

    #[test]
    fn test_partial_config() {
        let toml_str = r#"
[listen]
auto = ["only_one"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(config.api.token.is_none());
        assert_eq!(config.listen.auto, vec!["only_one"]);
        assert_eq!(config.timing.at_delay_ms, 300); // 默认值
    }

    #[test]
    fn test_empty_config() {
        let config: AppConfig = toml::from_str("").unwrap();
        assert!(config.api.token.is_none());
        assert!(config.listen.auto.is_empty());
        assert_eq!(config.timing.at_delay_ms, 300);
    }

    #[test]
    fn test_save_listen_list_replaces_inline() {
        let dir = std::env::temp_dir();
        let path = dir.join("mimicwx_test_save_inline.toml");
        let original = r#"# 配置文件
[api]
token = "abc"

[listen]
auto = ["old1", "old2"]

[timing]
at_delay_ms = 300
"#;
        std::fs::write(&path, original).unwrap();

        save_listen_list(&path, &["new1".into(), "new2".into(), "new3".into()]);

        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("auto = [\"new1\",\"new2\",\"new3\"]"));
        assert!(!saved.contains("old1"));
        assert!(saved.contains("# 配置文件"));
        assert!(saved.contains("token = \"abc\""));
        assert!(saved.contains("at_delay_ms = 300"));
    }

    #[test]
    fn test_save_listen_list_multiline_array() {
        let dir = std::env::temp_dir();
        let path = dir.join("mimicwx_test_save_multiline.toml");
        let original = "[listen]\nauto = [\n  \"a\",\n  \"b\",\n]\n";
        std::fs::write(&path, original).unwrap();

        save_listen_list(&path, &["x".into()]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("auto = [\"x\"]"));
        assert!(!saved.contains("\"a\""));
    }

    #[test]
    fn test_save_listen_list_empty() {
        let dir = std::env::temp_dir();
        let path = dir.join("mimicwx_test_save_empty.toml");
        let original = "[listen]\nauto = [\"to_remove\"]\n";
        std::fs::write(&path, original).unwrap();

        save_listen_list(&path, &[]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("auto = []"));
        assert!(!saved.contains("to_remove"));
    }

    #[test]
    fn test_save_listen_list_preserves_comments() {
        let dir = std::env::temp_dir();
        let path = dir.join("mimicwx_test_save_comments.toml");
        let original = r#"# 这是注释 auto = ["fake"]
[listen]
# auto = ["also_fake"]
auto = ["real"]

# 尾部注释
"#;
        std::fs::write(&path, original).unwrap();

        save_listen_list(&path, &["updated".into()]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# 这是注释 auto = [\"fake\"]"));
        assert!(saved.contains("# auto = [\"also_fake\"]"));
        assert!(saved.contains("auto = [\"updated\"]"));
        assert!(!saved.contains("\"real\""));
        assert!(saved.contains("# 尾部注释"));
    }

    #[test]
    fn test_save_listen_list_append_if_missing() {
        let dir = std::env::temp_dir();
        let path = dir.join("mimicwx_test_save_append.toml");
        let original = "[api]\ntoken = \"x\"\n";
        std::fs::write(&path, original).unwrap();

        save_listen_list(&path, &["appended".into()]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("[listen]"));
        assert!(saved.contains("auto = [\"appended\"]"));
    }
}
