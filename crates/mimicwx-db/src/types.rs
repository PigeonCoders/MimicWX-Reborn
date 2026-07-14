//! 数据库类型定义
//!
//! 联系人、群成员、会话、消息内容等结构体。

use serde::Serialize;

/// 联系人信息
#[derive(Debug, Clone, Serialize)]
pub struct ContactInfo {
    pub username: String,
    pub nick_name: String,
    pub remark: String,
    pub alias: String,
    /// 优先显示名: remark > nick_name > username
    pub display_name: String,
}

/// 群成员信息
#[derive(Debug, Clone, Serialize)]
pub struct GroupMemberInfo {
    /// 成员 wxid
    pub wxid: String,
    /// 微信昵称
    pub nick_name: String,
    /// 显示名 (优先: 备注 > 昵称 > wxid)
    pub display_name: String,
}

/// 会话信息 (来自数据库)
#[derive(Debug, Clone, Serialize)]
pub struct DbSessionInfo {
    pub username: String,
    pub display_name: String,
    pub unread_count: i32,
    pub summary: String,
    pub last_timestamp: i64,
    pub last_msg_sender: String,
}

/// 结构化消息内容 (按 msg_type 解析)
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum MsgContent {
    /// 纯文本 (msg_type=1)
    Text { text: String },
    /// 图片 (msg_type=3)
    Image {
        path: Option<String>,
        md5: Option<String>,
        length: Option<u64>,
        width: Option<u32>,
        height: Option<u32>,
    },
    /// 语音 (msg_type=34)
    Voice {
        duration_ms: Option<u32>,
        voice_url: Option<String>,
        aeskey: Option<String>,
    },
    /// 视频 (msg_type=43)
    Video {
        thumb_path: Option<String>,
        cdn_video_url: Option<String>,
        aeskey: Option<String>,
        length: Option<u64>,
        play_length: Option<u32>,
        width: Option<u32>,
        height: Option<u32>,
    },
    /// 表情包 (msg_type=47)
    Emoji { url: Option<String> },
    /// 链接/小程序 (msg_type=49, subtype != 6)
    App { title: Option<String>, desc: Option<String>, url: Option<String>, app_type: Option<i32> },
    /// 文件 (msg_type=49, subtype=6)
    File {
        title: Option<String>,
        file_size: Option<u64>,
        file_ext: Option<String>,
        md5: Option<String>,
    },
    /// 名片 (msg_type=42)
    ContactCard {
        nickname: Option<String>,
        username: Option<String>,
        avatar_url: Option<String>,
    },
    /// 位置 (msg_type=48)
    Location {
        x: Option<f64>,
        y: Option<f64>,
        scale: Option<u32>,
        label: Option<String>,
        poiname: Option<String>,
    },
    /// 系统消息 (msg_type=10000/10002)
    System { text: String },
    /// 未知类型
    Unknown { raw: String, msg_type: i64 },
}

impl MsgContent {
    /// 消息类型的简短描述 (用于日志)
    #[allow(dead_code)]
    pub fn type_label(&self) -> &'static str {
        match self {
            Self::Text { .. } => "文本",
            Self::Image { .. } => "图片",
            Self::Voice { .. } => "语音",
            Self::Video { .. } => "视频",
            Self::Emoji { .. } => "表情",
            Self::App { .. } => "链接",
            Self::File { .. } => "文件",
            Self::ContactCard { .. } => "名片",
            Self::Location { .. } => "位置",
            Self::System { .. } => "系统",
            Self::Unknown { .. } => "未知",
        }
    }

    /// 日志预览文本
    pub fn preview(&self, max_len: usize) -> String {
        let text = match self {
            Self::Text { text } => text.clone(),
            Self::Image { .. } => "[图片]".into(),
            Self::Voice { duration_ms, .. } => {
                match duration_ms {
                    Some(ms) if *ms >= 1000 => format!("[语音 {}s]", ms / 1000),
                    Some(ms) if *ms > 0 => format!("[语音 {ms}ms]"),
                    _ => "[语音]".into(),
                }
            }
            Self::Video { .. } => "[视频]".into(),
            Self::Emoji { url, .. } => format!("[表情] {}", url.as_deref().unwrap_or("")),
            Self::App { title, desc, app_type, .. } => {
                let t = title.as_deref().unwrap_or("");
                let d = desc.as_deref().unwrap_or("");
                let label = match app_type.unwrap_or(0) {
                    3 => "音乐",
                    19 => "转发",
                    33 | 36 => "小程序",
                    2000 => "转账",
                    2001 => "红包",
                    _ => "链接",
                };
                if !t.is_empty() { format!("[{label}] {t}") }
                else if !d.is_empty() { format!("[{label}] {d}") }
                else { format!("[{label}]") }
            }
            Self::File { title, file_size, .. } => {
                let t = title.as_deref().unwrap_or("未知文件");
                match file_size {
                    Some(s) if *s >= 1024 * 1024 => format!("[文件] {} ({:.1}MB)", t, *s as f64 / 1024.0 / 1024.0),
                    Some(s) if *s >= 1024 => format!("[文件] {} ({:.1}KB)", t, *s as f64 / 1024.0),
                    Some(s) => format!("[文件] {} ({}B)", t, s),
                    None => format!("[文件] {}", t),
                }
            }
            Self::ContactCard { nickname, username, .. } => {
                let name = nickname.as_deref()
                    .or(username.as_deref())
                    .unwrap_or("未知");
                format!("[名片] {}", name)
            }
            Self::Location { poiname, label, x, y, .. } => {
                let name = poiname.as_deref()
                    .or(label.as_deref())
                    .unwrap_or("未知位置");
                if let (Some(lat), Some(lng)) = (x, y) {
                    format!("[位置] {} ({:.4},{:.4})", name, lat, lng)
                } else {
                    format!("[位置] {}", name)
                }
            }
            Self::System { text } => format!("[系统] {text}"),
            Self::Unknown { msg_type, .. } => format!("[type={msg_type}]"),
        };
        if text.len() > max_len {
            format!("{}...", &text[..text.floor_char_boundary(max_len)])
        } else {
            text
        }
    }
}

/// 数据库消息
#[derive(Debug, Clone, Serialize)]
pub struct DbMessage {
    pub local_id: i64,
    pub server_id: i64,
    pub create_time: i64,
    /// 原始 content 字符串 (向后兼容)
    pub content: String,
    /// 结构化解析结果
    pub parsed: MsgContent,
    pub msg_type: i64,
    /// 发言人 wxid (群聊中有意义)
    pub talker: String,
    /// 发言人显示名 (通过联系人缓存解析)
    pub talker_display_name: String,
    /// 所属会话
    pub chat: String,
    /// 所属会话显示名
    pub chat_display_name: String,
    /// 是否为自己发送的消息
    pub is_self: bool,
    /// 是否 @ 了自己 (基于 source 列的 atuserlist 精确匹配 wxid)
    pub is_at_me: bool,
    /// 被 @ 的 wxid 列表 (来自 source 列 <atuserlist>)
    pub at_user_list: Vec<String>,
}

/// 原始消息 (同步查询返回, 后续异步填充显示名)
#[derive(Debug)]
pub struct RawMsg {
    pub local_id: i64,
    pub server_id: i64,
    pub create_time: i64,
    pub content: String,
    pub msg_type: i64,
    pub talker: String,
    pub chat: String,
    pub status: i64,
    /// 消息元数据 XML (含 atuserlist 等)
    pub source: String,
}

/// 消息表结构元数据缓存 (避免每次查询重新执行 PRAGMA table_info)
#[derive(Debug, Clone)]
pub struct TableMeta {
    /// 表名
    pub table: String,
    /// 预编译的 SELECT SQL
    pub select_sql: String,
    /// ID 列名 (local_id / rowid)
    pub id_col: String,
}
