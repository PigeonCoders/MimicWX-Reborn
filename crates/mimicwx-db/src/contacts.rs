//! 联系人管理
//!
//! `impl DbManager` 的联系人/群成员方法: 刷新缓存、查询、群成员解析。
//! 所有 DB 操作在 `spawn_blocking` 中完成, 异步方法只操作 ArcSwap 快照。

use crate::{key, parser, types::*, wcdb, DbManager};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

impl DbManager {
    /// 加载/刷新联系人缓存 (spawn_blocking 中执行 DB 查询)
    pub async fn refresh_contacts(&self) -> Result<usize> {
        let key_bytes = self.key_bytes.clone();
        let key_hex = self.key_hex.clone();
        let dir = self.db_dir.clone();
        let conn_mutex = Arc::clone(&self.contact_conn);

        let contacts = tokio::task::spawn_blocking(move || -> Result<Vec<ContactInfo>> {
            let mut guard = conn_mutex.lock().map_err(|e| anyhow::anyhow!("contact_conn lock: {}", e))?;
            if guard.is_none() {
                *guard = Some(key::open_db(&key_hex, &key_bytes, &dir, "contact/contact.db")?);
                info!(target: "mimicwx::db", "contact.db 连接已建立");
            }
            let conn = guard.as_ref().unwrap();
            let mut stmt = conn.prepare(
                "SELECT username, nick_name, remark, alias FROM contact"
            )?;
            let result: Vec<ContactInfo> = stmt.query_map([], |row| {
                let username = wcdb::wcdb_get_text(row, 0);
                if username.is_empty() {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                let nick_name = wcdb::wcdb_get_text(row, 1);
                let remark = wcdb::wcdb_get_text(row, 2);
                let alias = wcdb::wcdb_get_text(row, 3);
                let display_name = if !remark.is_empty() {
                    remark.clone()
                } else if !nick_name.is_empty() {
                    nick_name.clone()
                } else {
                    username.clone()
                };
                Ok(ContactInfo { username, nick_name, remark, alias, display_name })
            })?.filter_map(|r| match r {
                Ok(c) => Some(c),
                Err(e) => { warn!(target: "mimicwx::contact", "行读取失败: {e}"); None }
            }).collect();
            Ok(result)
        }).await??;

        let count = contacts.len();
        {
            let mut new_map = HashMap::with_capacity(contacts.len());
            for c in contacts {
                new_map.insert(c.username.clone(), c);
            }
            self.contacts.store(Arc::new(new_map));
        }
        info!(target: "mimicwx::contact", "联系人缓存: {count} 条");

        // 从 chat_room 表补充群名
        let chatrooms = {
            let conn_mutex2 = Arc::clone(&self.contact_conn);
            tokio::task::spawn_blocking(move || -> Result<Vec<(String, String)>> {
                let guard = conn_mutex2.lock().map_err(|e| anyhow::anyhow!("contact_conn lock: {}", e))?;
                if let Some(conn) = guard.as_ref() {
                    let mut result = Vec::new();
                    if let Ok(mut stmt) = conn.prepare(
                        "SELECT cr.username, c.nick_name FROM chat_room cr \
                         LEFT JOIN contact c ON cr.username = c.username \
                         WHERE cr.username IS NOT NULL"
                    ) {
                        let rows: Vec<(String, String)> = stmt.query_map([], |row| {
                            let id = wcdb::wcdb_get_text(row, 0);
                            let name = wcdb::wcdb_get_text(row, 1);
                            Ok((id, name))
                        }).ok()
                        .map(|iter| iter.filter_map(|r| r.ok()).collect())
                        .unwrap_or_default();

                        for (id, name) in rows {
                            if !id.is_empty() && !name.is_empty() {
                                debug!(target: "mimicwx::contact", "群聊补充: {id} → {name}");
                                result.push((id, name));
                            }
                        }
                    }
                    Ok(result)
                } else {
                    Ok(vec![])
                }
            }).await.unwrap_or_else(|_| Ok(vec![])).unwrap_or_default()
        };

        if !chatrooms.is_empty() {
            let old = self.contacts.load();
            let mut new_map = (**old).clone();
            let mut added = 0usize;
            for (chatroom_id, nick_name) in chatrooms {
                if !new_map.contains_key(&chatroom_id) {
                    new_map.insert(chatroom_id.clone(), ContactInfo {
                        username: chatroom_id,
                        nick_name: nick_name.clone(),
                        remark: String::new(),
                        alias: String::new(),
                        display_name: nick_name,
                    });
                    added += 1;
                }
            }
            if added > 0 {
                self.contacts.store(Arc::new(new_map));
                debug!(target: "mimicwx::contact", "群聊名称补充: {added} 条");
            }
        }

        // 解析当前账号的显示名
        if !self.self_wxid.is_empty() {
            let name = self.contacts.load()
                .get(&self.self_wxid)
                .map(|c| c.display_name.clone());
            if let Some(name) = name {
                debug!(target: "mimicwx::contact", "当前账号: {name} ({})", self.self_wxid);
                *self.self_display_name.write().await = name;
            }
        }

        Ok(count)
    }

    /// 获取联系人列表
    pub async fn get_contacts(&self) -> Vec<ContactInfo> {
        self.contacts.load().values().cloned().collect()
    }

    /// 获取群成员列表
    ///
    /// 从 contact.db 的 chat_room 表读取 ext_buffer (protobuf),
    /// 解析成员 wxid 列表, 再用联系人缓存解析显示名。
    pub async fn get_group_members(&self, chatroom_id: &str) -> Result<Vec<GroupMemberInfo>> {
        let conn_mutex = Arc::clone(&self.contact_conn);
        let chatroom = chatroom_id.to_string();

        let member_wxids = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
            let guard = conn_mutex.lock().map_err(|e| anyhow::anyhow!("contact_conn lock: {}", e))?;
            let conn = guard.as_ref().ok_or_else(|| anyhow::anyhow!("contact.db 未连接"))?;

            let ext_buffer: Vec<u8> = conn.query_row(
                "SELECT IFNULL(ext_buffer, X'') FROM chat_room WHERE username = ?",
                [&chatroom],
                |row| row.get(0),
            ).unwrap_or_default();

            if ext_buffer.is_empty() {
                info!(target: "mimicwx::db", "chat_room ext_buffer 为空: {}", chatroom);
                return Ok(vec![]);
            }

            let wxids = parser::parse_ext_buffer_wxids(&ext_buffer);
            info!(target: "mimicwx::db", "群 {} 解析到 {} 个成员", chatroom, wxids.len());
            Ok(wxids)
        }).await??;

        if member_wxids.is_empty() {
            return Ok(vec![]);
        }

        let contacts = self.contacts.load();
        let members: Vec<GroupMemberInfo> = member_wxids.into_iter().map(|wxid| {
            let (nick_name, display_name) = match contacts.get(&wxid) {
                Some(c) => (c.nick_name.clone(), c.display_name.clone()),
                None => (String::new(), wxid.clone()),
            };
            GroupMemberInfo { wxid, nick_name, display_name }
        }).collect();

        Ok(members)
    }

    /// 通过 username 获取显示名 (内部辅助)
    pub(crate) async fn resolve_name(&self, username: &str) -> String {
        self.contacts.load()
            .get(username)
            .map(|c| c.display_name.clone())
            .unwrap_or_else(|| username.to_string())
    }
}
