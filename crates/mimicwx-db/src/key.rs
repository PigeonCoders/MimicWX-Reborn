//! 密钥管理
//!
//! SQLCipher/WCDB 密钥传递: 支持已派生 (48字节) 和原始 (32字节) 两种密钥格式。
//! 三级密钥匹配: 专属密钥 → 默认密钥 → 暴力匹配。

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info, trace};

/// SQLite busy_timeout (ms)
pub const DB_BUSY_TIMEOUT_MS: u32 = 5000;

// =====================================================================
// FFI: sqlite3_key (WCDB 密钥传递方式)
// =====================================================================

extern "C" {
    /// WCDB 使用 sqlite3_key() C API 传递 raw key (非 PRAGMA key).
    /// SQLCipher 会对这个 key 做 PBKDF2 派生.
    fn sqlite3_key(
        db: *mut std::ffi::c_void,
        key: *const u8,
        key_len: std::ffi::c_int,
    ) -> std::ffi::c_int;
}

/// hex 字符串转字节
pub fn hex_to_bytes(hex: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(hex.len() % 2 == 0, "hex 长度必须为偶数");
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .with_context(|| format!("无效 hex 字符: {}", &hex[i..i + 2]))
        })
        .collect()
}

/// 从 JSON 映射文件查找数据库专属密钥
pub fn lookup_db_key(db_name: &str) -> Option<String> {
    let map = read_keys_json()?;
    if let Some(key) = map.get(db_name) {
        return Some(key.clone());
    }
    let basename = Path::new(db_name)
        .file_name().and_then(|f| f.to_str()).unwrap_or("");
    for (k, v) in &map {
        if k.ends_with(basename) { return Some(v.clone()); }
    }
    None
}

/// 读取 wechat_keys.json (优先持久化路径, 回退 /tmp)
pub fn read_keys_json() -> Option<HashMap<String, String>> {
    for path in &["/home/wechat/.xwechat/wechat_keys.json", "/tmp/wechat_keys.json"] {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(map) = serde_json::from_str(&content) {
                return Some(map);
            }
        }
    }
    None
}

/// 获取 wechat_keys.json 中所有唯一密钥 (用于暴力匹配)
pub fn all_json_keys() -> Vec<String> {
    let map = match read_keys_json() {
        Some(m) => m,
        None => return vec![],
    };
    let mut seen = std::collections::HashSet::new();
    map.into_values().filter(|v| seen.insert(v.clone())).collect()
}

/// 用指定密钥尝试打开加密数据库
pub fn try_open_db_with_key(path: &Path, db_name: &str, key_hex: &str, key_bytes: &[u8]) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ).with_context(|| format!("打开数据库失败: {}", path.display()))?;

    if key_bytes.len() == 48 {
        // 已派生密钥: PRAGMA key = "x'<96hex>'" 跳过 PBKDF2
        let pragma = format!("PRAGMA key = \"x'{}'\";", key_hex);
        conn.execute_batch(&pragma)
            .with_context(|| format!("PRAGMA key 失败: {}", db_name))?;
    } else {
        // 原始密钥: sqlite3_key() + PBKDF2 派生
        let rc = unsafe {
            let handle = conn.handle();
            sqlite3_key(
                handle as *mut std::ffi::c_void,
                key_bytes.as_ptr(),
                key_bytes.len() as std::ffi::c_int,
            )
        };
        anyhow::ensure!(rc == 0, "sqlite3_key() 失败, rc={}", rc);
    }

    conn.execute_batch("PRAGMA cipher_compatibility = 4;")?;
    conn.execute_batch("PRAGMA wal_autocheckpoint = 0;")?;
    conn.execute_batch("PRAGMA query_only = ON;")?;
    conn.execute_batch(&format!("PRAGMA busy_timeout = {};", DB_BUSY_TIMEOUT_MS))?;

    let count: i32 = conn.query_row(
        "SELECT count(*) FROM sqlite_master", [], |row| row.get(0),
    ).with_context(|| format!("数据库解密验证失败: {}", db_name))?;

    trace!(target: "mimicwx::db", "{db_name} 解密成功, {count} 个表");
    Ok(conn)
}

/// 打开加密数据库 (只读模式, 自动尝试专属密钥 → 默认密钥 → 暴力匹配)
pub fn open_db(key_hex: &str, key_bytes: &[u8], db_dir: &Path, db_name: &str) -> Result<Connection> {
    let path = db_dir.join(db_name);
    anyhow::ensure!(path.exists(), "数据库不存在: {}", path.display());

    // 1. 查找此数据库的专属密钥
    if let Some(db_key) = lookup_db_key(db_name) {
        let bytes = hex_to_bytes(&db_key).unwrap_or_default();
        if let Ok(conn) = try_open_db_with_key(&path, db_name, &db_key, &bytes) {
            return Ok(conn);
        }
        debug!(target: "mimicwx::key", "{db_name} 专属密钥失败, 尝试其他密钥");
    }

    // 2. 尝试默认密钥
    if let Ok(conn) = try_open_db_with_key(&path, db_name, key_hex, key_bytes) {
        return Ok(conn);
    }

    // 3. 暴力匹配: 尝试 wechat_keys.json 中的所有密钥
    let all_keys = all_json_keys();
    for candidate in &all_keys {
        if candidate == key_hex { continue; }
        let bytes = match hex_to_bytes(candidate) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Ok(conn) = try_open_db_with_key(&path, db_name, candidate, &bytes) {
            info!(target: "mimicwx::key", "{db_name} 通过暴力匹配找到密钥");
            return Ok(conn);
        }
    }

    anyhow::bail!("数据库解密验证失败: {} (已尝试 {} 个密钥)", db_name, all_keys.len() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_to_bytes_valid() {
        assert_eq!(hex_to_bytes("48656c6c6f").unwrap(), b"Hello");
        assert_eq!(hex_to_bytes("00ff").unwrap(), vec![0x00, 0xFF]);
    }

    #[test]
    fn test_hex_to_bytes_odd_length() {
        assert!(hex_to_bytes("abc").is_err());
    }

    #[test]
    fn test_hex_to_bytes_invalid_char() {
        assert!(hex_to_bytes("xy").is_err());
    }

    #[test]
    fn test_hex_to_bytes_empty() {
        assert_eq!(hex_to_bytes("").unwrap(), Vec::<u8>::new());
    }
}
