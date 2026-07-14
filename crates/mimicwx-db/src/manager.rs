//! DbManager — 数据库管理主结构
//!
//! 管理 SQLCipher 加密数据库连接池, 提供增量消息获取、会话查询、
//! 发送验证等功能。所有 DB 操作在 `spawn_blocking` 中完成,
//! 异步方法只操作缓存 (ArcSwap 联系人快照、Mutex 高水位线)。

use crate::{key, parser, types::*, wcdb};
use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// 发送验证超时
const SEND_VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub struct DbManager {
    /// 密钥 hex 字符串 (96 hex = 已派生, 64 hex = 原始)
    pub(crate) key_hex: String,
    pub(crate) key_bytes: Vec<u8>,
    /// 数据库存储目录
    pub(crate) db_dir: PathBuf,
    /// 当前登录账号的 wxid (从 db_dir 路径提取)
    pub(crate) self_wxid: String,
    /// ensure_msg_conns 空扫描计数 (抑制重复日志)
    pub(crate) rescan_count: std::sync::atomic::AtomicU32,
    /// 当前账号的显示名 (从联系人库查询, 默认 "我")
    pub(crate) self_display_name: tokio::sync::RwLock<String>,
    /// 联系人缓存: username → ContactInfo (ArcSwap 快照, 读取零竞争)
    pub(crate) contacts: ArcSwap<HashMap<String, ContactInfo>>,
    /// 高水位线: "db_name::表名" → 最大 local_id
    pub(crate) watermarks: Mutex<HashMap<String, i64>>,
    /// 持久化 message_N.db 连接池
    pub(crate) msg_conns: std::sync::Mutex<HashMap<String, Arc<std::sync::Mutex<Connection>>>>,
    /// 持久化 contact.db 连接
    pub(crate) contact_conn: Arc<std::sync::Mutex<Option<Connection>>>,
    /// 持久化 session.db 连接
    pub(crate) session_conn: Arc<std::sync::Mutex<Option<Connection>>>,
    /// 消息表结构元数据缓存
    pub(crate) table_meta_cache: std::sync::Mutex<HashMap<String, TableMeta>>,
    /// WAL 变化广播通知
    pub(crate) wal_notify: tokio::sync::broadcast::Sender<()>,
    /// 自发消息广播: (content, local_id)
    pub(crate) sent_content_tx: tokio::sync::broadcast::Sender<(String, i64)>,
}

impl DbManager {
    /// 创建 DbManager
    pub fn new(key_hex: String, db_dir: PathBuf) -> Result<Self> {
        let key_bytes = key::hex_to_bytes(&key_hex)
            .context("密钥 hex 格式错误")?;
        anyhow::ensure!(key_bytes.len() == 32 || key_bytes.len() == 48,
            "密钥长度必须为 32 或 48 字节, 实际: {}", key_bytes.len());

        info!(target: "mimicwx::db", "DbManager 初始化: {}", db_dir.display());

        let self_wxid = db_dir.components()
            .filter_map(|c| c.as_os_str().to_str())
            .find(|s| s.starts_with("wxid_"))
            .map(|s| {
                if let Some(pos) = s.rfind('_') {
                    let suffix = &s[pos+1..];
                    if suffix.len() <= 6
                        && suffix.len() >= 2
                        && suffix.chars().all(|c| c.is_ascii_alphanumeric())
                        && !suffix.starts_with("wxid")
                    {
                        return s[..pos].to_string();
                    }
                }
                s.to_string()
            })
            .unwrap_or_default();
        if !self_wxid.is_empty() {
            debug!(target: "mimicwx::db", "当前账号: {self_wxid}");
        }

        let mut conns = HashMap::new();
        let msg_dir = db_dir.join("message");
        if msg_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&msg_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if wcdb::is_message_db(&name) {
                        let rel_path = format!("message/{}", name);
                        match key::open_db(&key_hex, &key_bytes, &db_dir, &rel_path) {
                            Ok(conn) => {
                                info!(target: "mimicwx::db", "{name} 连接已建立");
                                conns.insert(rel_path, Arc::new(std::sync::Mutex::new(conn)));
                            }
                            Err(e) => {
                                warn!(target: "mimicwx::db", "{name} 暂不可用 (查询时重试): {e}");
                            }
                        }
                    }
                }
            }
        }
        if conns.is_empty() {
            warn!(target: "mimicwx::db", "未发现 message 数据库 (首次查询时重试)");
        } else {
            info!(target: "mimicwx::db", "已连接 {} 个消息数据库", conns.len());
        }

        let (wal_tx, _) = tokio::sync::broadcast::channel::<()>(64);
        let (sent_tx, _) = tokio::sync::broadcast::channel::<(String, i64)>(32);
        Ok(Self {
            key_hex: key_hex.clone(),
            key_bytes,
            db_dir,
            self_wxid,
            self_display_name: tokio::sync::RwLock::new("我".to_string()),
            contacts: ArcSwap::from_pointee(HashMap::new()),
            watermarks: Mutex::new(HashMap::new()),
            msg_conns: std::sync::Mutex::new(conns),
            contact_conn: Arc::new(std::sync::Mutex::new(None)),
            session_conn: Arc::new(std::sync::Mutex::new(None)),
            table_meta_cache: std::sync::Mutex::new(HashMap::new()),
            wal_notify: wal_tx,
            sent_content_tx: sent_tx,
            rescan_count: std::sync::atomic::AtomicU32::new(0),
        })
    }

    /// 确保至少有一个 message 数据库连接可用 (如为空则重新扫描)
    pub(crate) fn ensure_msg_conns(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Arc<std::sync::Mutex<Connection>>>>> {
        let mut guard = self.msg_conns.lock().map_err(|e| anyhow::anyhow!("msg_conns lock poisoned: {}", e))?;
        if guard.is_empty() {
            let count = self.rescan_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if count == 0 {
                info!(target: "mimicwx::db", "重新扫描 message 数据库");
            } else {
                debug!(target: "mimicwx::db", "重新扫描 message 数据库 ({}/{})", count + 1, count + 1);
            }
            let msg_dir = self.db_dir.join("message");
            if msg_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&msg_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if wcdb::is_message_db(&name) {
                            let rel_path = format!("message/{}", name);
                            if !guard.contains_key(&rel_path) {
                                if let Ok(conn) = key::open_db(&self.key_hex, &self.key_bytes, &self.db_dir, &rel_path) {
                                    info!(target: "mimicwx::db", "{name} 连接已建立");
                                    guard.insert(rel_path, Arc::new(std::sync::Mutex::new(conn)));
                                }
                            }
                        }
                    }
                }
            }
            if !guard.is_empty() {
                self.rescan_count.store(0, std::sync::atomic::Ordering::Relaxed);
            }
            anyhow::ensure!(!guard.is_empty(), "无可用的 message 数据库");
        }
        Ok(guard)
    }

    /// 获取会话列表
    pub async fn get_sessions(&self) -> Result<Vec<DbSessionInfo>> {
        let key_bytes = self.key_bytes.clone();
        let key_hex = self.key_hex.clone();
        let dir = self.db_dir.clone();
        let conn_mutex = Arc::clone(&self.session_conn);

        let rows = tokio::task::spawn_blocking(move || -> Result<Vec<(String, i32, String, i64, String)>> {
            let mut guard = conn_mutex.lock().map_err(|e| anyhow::anyhow!("session_conn lock: {}", e))?;
            if guard.is_none() {
                *guard = Some(key::open_db(&key_hex, &key_bytes, &dir, "session/session.db")?);
                info!(target: "mimicwx::db", "session.db 连接已建立");
            }
            let conn = guard.as_ref().unwrap();
            let mut stmt = conn.prepare(
                "SELECT username, unread_count, summary, last_timestamp, last_msg_sender \
                 FROM SessionTable ORDER BY sort_timestamp DESC"
            )?;
            let result = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i32>>(1)?.unwrap_or(0),
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                ))
            })?.filter_map(|r| r.ok()).collect();
            Ok(result)
        }).await??;

        let mut sessions = Vec::with_capacity(rows.len());
        for (username, unread_count, summary, last_timestamp, last_msg_sender) in rows {
            let display_name = self.resolve_name(&username).await;
            sessions.push(DbSessionInfo {
                username, display_name, unread_count, summary, last_timestamp, last_msg_sender,
            });
        }
        Ok(sessions)
    }

    /// 获取新消息 (遍历所有 message_N.db 持久连接)
    pub async fn get_new_messages(&self) -> Result<Vec<DbMessage>> {
        let current_watermarks = self.watermarks.lock().await.clone();

        let conn_arcs: Vec<(String, Arc<std::sync::Mutex<Connection>>)> = {
            let conns_guard = self.ensure_msg_conns()?;
            conns_guard.iter()
                .map(|(name, conn)| (name.clone(), Arc::clone(conn)))
                .collect()
        };

        let cached_meta: HashMap<String, TableMeta> = {
            self.table_meta_cache.lock()
                .map(|g| g.clone())
                .unwrap_or_default()
        };

        let (raw_msgs, new_watermarks, updated_meta) = tokio::task::spawn_blocking(move || -> Result<(Vec<RawMsg>, HashMap<String, i64>, HashMap<String, TableMeta>)> {
            let mut all_msgs = Vec::new();
            let mut wm = current_watermarks;
            let mut name2id_cache: HashMap<String, String> = HashMap::new();
            let mut meta_cache = cached_meta;

            for (db_name, conn_arc) in &conn_arcs {
                let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("conn lock: {}", e))?;
                let db_prefix = db_name.trim_start_matches("message/").trim_end_matches(".db");

                let tables = wcdb::discover_msg_tables(&conn);
                if tables.is_empty() { continue; }

                let mut table_metas = Vec::new();
                for table in &tables {
                    let cache_key = format!("{}::{}", db_name, table);
                    if let Some(cached) = meta_cache.get(&cache_key) {
                        table_metas.push(cached.clone());
                    } else {
                        if let Some(meta) = wcdb::build_single_table_meta(&conn, table) {
                            info!(target: "mimicwx::db", "新增表缓存 {} ({})", table, db_name);
                            meta_cache.insert(cache_key, meta.clone());
                            table_metas.push(meta);
                        }
                    }
                }

                for meta in &table_metas {
                    let wm_key = format!("{}::{}", db_prefix, meta.table);
                    let last_id = wm.get(&wm_key).copied().unwrap_or(0);

                    let mut stmt = match conn.prepare(&meta.select_sql) {
                        Ok(s) => s,
                        Err(e) => { warn!(target: "mimicwx::db", "查询 {} ({}) 失败: {e}", meta.table, db_name); continue; }
                    };
                    let msgs: Vec<(i64, i64, i64, String, i64, String, i64, String)> = match stmt
                        .query_map([last_id], |row| {
                            let local_id: i64 = row.get(0)?;
                            let svr_id: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
                            let ts: i64 = row.get::<_, Option<i64>>(2)?.unwrap_or(0);

                            let content = match row.get::<_, Option<String>>(3) {
                                Ok(s) => s.unwrap_or_default(),
                                Err(_) => {
                                    match row.get::<_, Option<Vec<u8>>>(3) {
                                        Ok(Some(bytes)) => wcdb::decompress_wcdb_content(&bytes),
                                        _ => String::new(),
                                    }
                                }
                            };

                            let msg_type: i64 = row.get::<_, Option<i64>>(4)?.unwrap_or(0);

                            let sender = match row.get::<_, Option<String>>(5) {
                                Ok(s) => s.unwrap_or_default(),
                                Err(_) => match row.get::<_, Option<Vec<u8>>>(5) {
                                    Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
                                    _ => String::new(),
                                }
                            };

                            let status: i64 = row.get::<_, Option<i64>>(6)?.unwrap_or(0);
                            let source = wcdb::wcdb_get_text(row, 7);

                            Ok((local_id, svr_id, ts, content, msg_type, sender, status, source))
                        }) {
                        Ok(rows) => rows.filter_map(|r| match r {
                            Ok(v) => Some(v),
                            Err(e) => { warn!(target: "mimicwx::db", "行解析失败: {e}"); None }
                        }).collect(),
                        Err(e) => { warn!(target: "mimicwx::db", "query_map {} ({}) 失败: {e}", meta.table, db_name); continue; }
                    };

                    if !msgs.is_empty() {
                        let chat = wcdb::resolve_chat_from_table(&meta.table, &conn, &mut name2id_cache);
                        let mut max_id = last_id;
                        for (local_id, server_id, create_time, content, msg_type, talker, status, source) in msgs {
                            all_msgs.push(RawMsg {
                                local_id, server_id, create_time, content, msg_type,
                                talker, chat: chat.clone(), status, source,
                            });
                            if local_id > max_id { max_id = local_id; }
                        }
                        wm.insert(wm_key.clone(), max_id);
                    }
                }
            }

            Ok((all_msgs, wm, meta_cache))
        }).await??;

        // 回写表结构缓存
        if let Ok(mut cache) = self.table_meta_cache.lock() {
            for (k, v) in updated_meta {
                cache.entry(k).or_insert(v);
            }
        }

        if !raw_msgs.is_empty() {
            *self.watermarks.lock().await = new_watermarks;
        }

        // 异步填充显示名
        let contacts_cache = self.contacts.load();
        let self_display = self.self_display_name.read().await.clone();
        let resolve = |username: &str| -> String {
            contacts_cache
                .get(username)
                .map(|c| c.display_name.clone())
                .unwrap_or_else(|| username.to_string())
        };

        let mut result = Vec::with_capacity(raw_msgs.len());
        for m in raw_msgs {
            let mut talker = m.talker;
            let mut content = m.content;

            // 群聊中发送人 wxid 嵌入在消息内容中: "wxid_xxx:\n实际消息"
            if talker.is_empty() && m.chat.contains("@chatroom") {
                if let Some(pos) = content.find(":\n") {
                    let prefix = &content[..pos];
                    if !prefix.is_empty() && !prefix.contains(' ') && prefix.len() < 50 {
                        talker = prefix.to_string();
                        content = content[pos + 2..].to_string();
                    }
                }
            }

            // status bit 1 (0x02): 1=收到, 0=自发; 系统消息排除
            let base_msg_type = (m.msg_type & 0xFFFF) as i32;
            let is_self = (m.status & 0x02) == 0
                && base_msg_type != 10000
                && base_msg_type != 10002;

            if talker.is_empty() {
                if is_self {
                    talker = self.self_wxid.clone();
                } else if !m.chat.contains("@chatroom") {
                    talker = m.chat.clone();
                }
            }

            let talker_display = if is_self {
                self_display.clone()
            } else {
                resolve(&talker)
            };
            let chat_display = resolve(&m.chat);

            let base_type = (m.msg_type & 0xFFFF) as i32;
            if base_type != 1 {
                let raw_preview = if content.len() > 200 {
                    format!("{}...", &content[..content.floor_char_boundary(200)])
                } else {
                    content.clone()
                };
                debug!(target: "mimicwx::msg", "type={} raw: {}", base_type, raw_preview);
            }
            let parsed = parser::parse_msg_content(m.msg_type, &content);

            let at_user_list: Vec<String> = parser::extract_xml_text(&m.source, "atuserlist")
                .map(|s| s.split(',')
                    .map(|w| w.trim().to_string())
                    .filter(|w| !w.is_empty())
                    .collect())
                .unwrap_or_default();
            let is_at_me = !self.self_wxid.is_empty()
                && at_user_list.iter().any(|w| w == &self.self_wxid);

            result.push(DbMessage {
                local_id: m.local_id,
                server_id: m.server_id,
                create_time: m.create_time,
                content: content.clone(),
                parsed,
                msg_type: m.msg_type,
                talker,
                talker_display_name: talker_display,
                chat: m.chat,
                chat_display_name: chat_display,
                is_self,
                is_at_me,
                at_user_list,
            });

            // 自发消息广播
            if is_self {
                let _ = self.sent_content_tx.send((content, m.local_id));
            }
        }
        drop(contacts_cache);

        for m in &result {
            let preview = m.parsed.preview(40);
            let icon = if m.is_self { "[send] →" } else { "" };
            if m.chat.contains("@chatroom") {
                info!(target: "mimicwx::msg", "{icon} [{}] {}({}): {}",
                    m.chat_display_name, m.talker_display_name, m.talker, preview);
            } else {
                info!(target: "mimicwx::msg", "{icon} {}({}): {}",
                    m.chat_display_name, m.talker, preview);
            }
        }
        Ok(result)
    }

    /// 标记所有已有消息为已读
    pub async fn mark_all_read(&self) -> Result<()> {
        let conn_arcs: Vec<(String, Arc<std::sync::Mutex<Connection>>)> = {
            let conns_guard = self.ensure_msg_conns()?;
            conns_guard.iter()
                .map(|(name, conn)| (name.clone(), Arc::clone(conn)))
                .collect()
        };

        let wm = tokio::task::spawn_blocking(move || -> Result<HashMap<String, i64>> {
            let mut watermarks = HashMap::new();
            let mut total_tables = 0;

            for (db_name, conn_arc) in &conn_arcs {
                let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("conn lock: {}", e))?;
                let db_prefix = db_name.trim_start_matches("message/").trim_end_matches(".db");

                let tables = wcdb::discover_msg_tables(&conn);
                for table in &tables {
                    if let Some(meta) = wcdb::build_single_table_meta(&conn, table) {
                        let wm_key = format!("{}::{}", db_prefix, table);
                        let sql = format!("SELECT MAX({}) FROM [{}]", meta.id_col, table);
                        if let Ok(max_id) = conn.query_row(&sql, [], |row| row.get::<_, Option<i64>>(0)) {
                            if let Some(id) = max_id {
                                watermarks.insert(wm_key, id);
                            }
                        }
                    }
                }
                total_tables += tables.len();
            }
            info!(target: "mimicwx::db", "已标记 {total_tables} 个表为已读 (跨 {} 个数据库)", conn_arcs.len());
            Ok(watermarks)
        }).await??;

        *self.watermarks.lock().await = wm;
        Ok(())
    }

    /// 通过数据库验证消息是否发送成功, 并返回 local_id (事件驱动)
    pub async fn verify_sent(&self, text: &str, mut sent_rx: tokio::sync::broadcast::Receiver<(String, i64)>) -> Option<i64> {
        let text_owned = text.to_string();

        let deadline = tokio::time::Instant::now() + SEND_VERIFY_TIMEOUT;
        loop {
            tokio::select! {
                result = sent_rx.recv() => {
                    match result {
                        Ok((content, local_id)) => {
                            let content_trimmed = content.trim();
                            if !content_trimmed.is_empty() && (
                                content_trimmed.contains(&text_owned)
                                || text_owned.contains(content_trimmed)
                            ) {
                                info!(target: "mimicwx::verify", "发送验证通过 (local_id={})", local_id);
                                return Some(local_id);
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            warn!(target: "mimicwx::verify", "自发消息广播通道已关闭");
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    warn!(target: "mimicwx::verify", "发送验证超时 (5s)");
                    break;
                }
            }
        }
        None
    }

    /// 订阅自发消息广播 (在发送前调用, 确保不丢失发送期间的事件)
    pub fn subscribe_sent(&self) -> tokio::sync::broadcast::Receiver<(String, i64)> {
        self.sent_content_tx.subscribe()
    }

    /// 订阅 WAL 变化通知
    #[allow(dead_code)]
    pub fn subscribe_wal_events(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.wal_notify.subscribe()
    }

    /// 启动 WAL 文件监听 (fanotify + PID 过滤, 在独立线程运行)
    pub fn spawn_wal_watcher(self: &Arc<Self>) -> tokio::sync::broadcast::Receiver<()> {
        let wal_tx = self.wal_notify.clone();
        let db_dir = self.db_dir.clone();

        std::thread::spawn(move || {
            if let Err(e) = crate::listener::wal_watch_loop(&db_dir, wal_tx) {
                tracing::error!(target: "mimicwx::db", "WAL 监听退出: {e}");
            }
        });

        info!(target: "mimicwx::db", "WAL 监听已启动 (fanotify + broadcast)");
        self.wal_notify.subscribe()
    }
}
