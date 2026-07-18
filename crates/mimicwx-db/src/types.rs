//! 数据库类型定义
//!
//! 联系人、群成员、会话、消息内容等结构体。

use serde::Serialize;

/// App 消息子类型 (msg_type=49 的 `<type>` 字段)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    Music,
    Link,
    ChatRecord,
    MiniProgram,
    Pat,
    Announcement,
    Gift,
    Transfer,
    RedPacket,
    Unknown,
}

impl AppKind {
    pub fn from_app_type(app_type: Option<i32>) -> Self {
        match app_type {
            Some(3) => Self::Music,
            Some(4 | 5 | 49) => Self::Link,
            Some(19) => Self::ChatRecord,
            Some(33 | 36) => Self::MiniProgram,
            Some(62) => Self::Pat,
            Some(87) => Self::Announcement,
            Some(115) => Self::Gift,
            Some(2000) => Self::Transfer,
            Some(2001) => Self::RedPacket,
            _ => Self::Unknown,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Music => "音乐",
            Self::Link => "链接",
            Self::ChatRecord => "聊天记录",
            Self::MiniProgram => "小程序",
            Self::Pat => "拍一拍",
            Self::Announcement => "群公告",
            Self::Gift => "微信礼物",
            Self::Transfer => "转账",
            Self::RedPacket => "红包",
            Self::Unknown => "App",
        }
    }
}

/// 合并转发聊天记录中的单条消息 (msg_type=49, subtype=19)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatRecordItem {
    pub datatype: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_desc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_head_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_ext: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cdn_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aes_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_width: Option<u32>,
    /// 微信原始 duration 值；语音通常为毫秒。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<u64>,
}

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
    /// App 消息 (msg_type=49，文件和引用消息使用独立 variant)
    App {
        title: Option<String>,
        desc: Option<String>,
        url: Option<String>,
        app_type: Option<i32>,
        kind: AppKind,
        /// subtype=19 的 `<recorditem>` CDATA 解包后的内层 XML
        #[serde(skip_serializing_if = "Option::is_none")]
        record_item_xml: Option<String>,
        /// subtype=19 内按原顺序解析出的聊天记录条目
        #[serde(skip_serializing_if = "Vec::is_empty")]
        record_items: Vec<ChatRecordItem>,
    },
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
    /// 引用消息 (msg_type=49 sub=57, 或 msg_type=244813135921)
    Quote {
        /// 被引用的内容预览（如 "[图片]" / "你好" / "[链接] 标题"）
        quoted_content: String,
        /// 被引用消息的发送者名称
        quoted_sender: Option<String>,
        /// 被引用图片的 MD5（仅当引用图片时有值）
        image_md5: Option<String>,
        /// 被引用表情的 MD5
        emoji_md5: Option<String>,
        /// 被引用表情的 CDN URL
        emoji_cdn_url: Option<String>,
        /// 引用时的必填附言（来自外层 <title>）
        comment: String,
    },
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
            Self::App { kind, .. } => kind.label(),
            Self::File { .. } => "文件",
            Self::ContactCard { .. } => "名片",
            Self::Location { .. } => "位置",
            Self::System { .. } => "系统",
            Self::Quote { .. } => "引用",
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
            Self::App { title, desc, kind, .. } => {
                let t = title.as_deref().unwrap_or("");
                let d = desc.as_deref().unwrap_or("");
                let label = kind.label();
                if !t.is_empty() {
                    format!("[{label}] {t}")
                } else if !d.is_empty() {
                    format!("[{label}] {d}")
                } else {
                    format!("[{label}]")
                }
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
            Self::Quote { comment, quoted_content, quoted_sender, .. } => {
                let quoted = match quoted_sender {
                    Some(sender) => format!("{sender}: {quoted_content}"),
                    None => quoted_content.clone(),
                };
                format!("{comment} [引用 {quoted}]")
            }
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
