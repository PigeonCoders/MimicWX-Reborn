//! WCDB 兼容层
//!
//! WCDB 压缩可能导致 TEXT 列实际存储为 Zstd BLOB。
//! 提供自适应读取、表结构发现、表名→会话名解析等功能。

use crate::types::TableMeta;
use rusqlite::Connection;
use std::collections::HashMap;
use tracing::debug;

/// WCDB Zstd BLOB 解压: 检测 Zstd magic 0x28B52FFD, 解压后返回 UTF-8 字符串
pub fn decompress_wcdb_content(blob: &[u8]) -> String {
    if blob.len() >= 4 && blob[0] == 0x28 && blob[1] == 0xB5 && blob[2] == 0x2F && blob[3] == 0xFD {
        match zstd::decode_all(blob) {
            Ok(data) => return String::from_utf8_lossy(&data).to_string(),
            Err(e) => tracing::warn!(target: "mimicwx::db", "Zstd 解压失败: {e}"),
        }
    }
    String::from_utf8_lossy(blob).to_string()
}

/// WCDB 兼容读取: 先尝试 TEXT, 失败则 BLOB + Zstd 解压
/// (WCDB 压缩可能导致 TEXT 列实际存储为 BLOB)
pub fn wcdb_get_text(row: &rusqlite::Row, idx: usize) -> String {
    match row.get::<_, Option<String>>(idx) {
        Ok(s) => s.unwrap_or_default(),
        Err(_) => match row.get::<_, Option<Vec<u8>>>(idx) {
            Ok(Some(bytes)) => decompress_wcdb_content(&bytes),
            _ => String::new(),
        },
    }
}

/// 判断文件名是否为 message_N.db 格式 (N 是数字)
pub fn is_message_db(name: &str) -> bool {
    if let Some(rest) = name.strip_prefix("message_") {
        if let Some(num_part) = rest.strip_suffix(".db") {
            return !num_part.is_empty() && num_part.chars().all(|c| c.is_ascii_digit());
        }
    }
    false
}

/// 查询 sqlite_master 获取消息表列表 (每次调用, 发现新表)
pub fn discover_msg_tables(conn: &Connection) -> Vec<String> {
    match conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND \
         (name LIKE 'ChatMsg_%' OR name LIKE 'MSG_%' OR name LIKE 'Chat_%')"
    ) {
        Ok(mut stmt) => {
            stmt.query_map([], |row| row.get(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        }
        Err(_) => Vec::new(),
    }
}

/// 对单个消息表执行 PRAGMA table_info → 构建 TableMeta (仅新表调用一次)
pub fn build_single_table_meta(conn: &Connection, table: &str) -> Option<TableMeta> {
    let pragma_sql = format!("PRAGMA table_info({})", table);
    let mut pragma_stmt = conn.prepare(&pragma_sql).ok()?;
    let columns: Vec<String> = pragma_stmt
        .query_map([], |row| row.get::<_, String>(1))
        .ok()?
        .filter_map(|r| r.ok()).collect();

    let id_col = columns.iter().find(|c| {
        c.eq_ignore_ascii_case("local_id") || c.eq_ignore_ascii_case("localId")
            || c.eq_ignore_ascii_case("rowid")
    }).cloned().unwrap_or_else(|| "rowid".to_string());

    let time_col = columns.iter().find(|c| {
        c.eq_ignore_ascii_case("create_time") || c.eq_ignore_ascii_case("createTime")
    }).cloned();

    let content_col = columns.iter().find(|c| {
        c.eq_ignore_ascii_case("message_content")
            || c.eq_ignore_ascii_case("content")
            || c.eq_ignore_ascii_case("msgContent")
            || c.eq_ignore_ascii_case("compress_content")
    }).cloned();

    let type_col = columns.iter().find(|c| {
        c.eq_ignore_ascii_case("local_type")
            || c.eq_ignore_ascii_case("type")
            || c.eq_ignore_ascii_case("msgType")
    }).cloned();

    let talker_col = columns.iter().find(|c| {
        c.eq_ignore_ascii_case("real_sender_id")
            || c.eq_ignore_ascii_case("talker")
            || c.eq_ignore_ascii_case("talkerId")
    }).cloned();

    let svr_col = columns.iter().find(|c| {
        c.eq_ignore_ascii_case("server_id") || c.eq_ignore_ascii_case("svrid")
            || c.eq_ignore_ascii_case("msgSvrId")
    }).cloned();

    let content_sel = content_col.as_deref()?;
    let time_sel = time_col.as_deref().unwrap_or("0");
    let type_sel = type_col.as_deref().unwrap_or("0");
    let talker_sel = talker_col.as_deref().unwrap_or("''");
    let svr_sel = svr_col.as_deref().unwrap_or("0");

    let status_col = columns.iter().find(|c| {
        c.eq_ignore_ascii_case("status")
    }).cloned();
    let status_sel = status_col.as_deref().unwrap_or("0");

    let source_col = columns.iter().find(|c| {
        c.eq_ignore_ascii_case("source")
    }).cloned();
    let source_sel = source_col.as_deref().unwrap_or("''");

    let select_sql = format!(
        "SELECT {id}, {svr}, {time}, {content}, {typ}, {talker}, {status}, {source} \
         FROM [{tbl}] WHERE {id} > ?1 ORDER BY {id} ASC",
        id = id_col, svr = svr_sel, time = time_sel,
        content = content_sel, typ = type_sel, talker = talker_sel,
        status = status_sel, source = source_sel, tbl = table,
    );

    Some(TableMeta {
        table: table.to_string(),
        select_sql,
        id_col,
    })
}

/// 从消息表名解析会话 username
/// ChatMsg_<rowid> -> Name2Id.user_name WHERE rowid = <id>
/// Msg_<hash> -> MD5(Name2Id.user_name) == hash (使用缓存 O(1) 查找)
pub fn resolve_chat_from_table(
    table_name: &str,
    conn: &Connection,
    cache: &mut HashMap<String, String>,
) -> String {
    if let Some(suffix) = table_name.strip_prefix("ChatMsg_") {
        if let Ok(id) = suffix.parse::<i64>() {
            let sql = "SELECT user_name FROM Name2Id WHERE rowid = ?1";
            if let Ok(name) = conn.query_row(sql, [id], |row| row.get::<_, String>(0)) {
                debug!(target: "mimicwx::db", "ChatMsg rowid={id} → {name}");
                return name;
            }
        }
    }

    if let Some(hash) = table_name.strip_prefix("Msg_")
        .or_else(|| table_name.strip_prefix("MSG_"))
        .or_else(|| table_name.strip_prefix("Chat_"))
    {
        if cache.is_empty() {
            if let Ok(mut stmt) = conn.prepare("SELECT user_name FROM Name2Id") {
                if let Ok(names) = stmt.query_map([], |row| row.get::<_, String>(0)) {
                    for name in names.flatten() {
                        let name_hash = format!("{:x}", md5::compute(name.as_bytes()));
                        cache.insert(name_hash, name);
                    }
                }
            }
            debug!(target: "mimicwx::db", "Name2Id 缓存: {} 条", cache.len());
        }

        if let Some(name) = cache.get(hash) {
            debug!(target: "mimicwx::db", "Msg hash={hash} → {name}");
            return name.clone();
        }
        debug!(target: "mimicwx::db", "hash={hash} 未匹配 Name2Id");
    }

    debug!(target: "mimicwx::db", "无法解析会话名: {table_name}");
    table_name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_message_db() {
        assert!(is_message_db("message_0.db"));
        assert!(is_message_db("message_1.db"));
        assert!(is_message_db("message_10.db"));
        assert!(!is_message_db("message_fts.db"));
        assert!(!is_message_db("message_resource.db"));
        assert!(!is_message_db("contact.db"));
        assert!(!is_message_db("message_.db"));
        assert!(!is_message_db("message.db"));
    }

    #[test]
    fn test_decompress_non_zstd() {
        let data = b"plain text";
        assert_eq!(decompress_wcdb_content(data), "plain text");
    }

    #[test]
    fn test_decompress_empty() {
        assert_eq!(decompress_wcdb_content(&[]), "");
    }
}
