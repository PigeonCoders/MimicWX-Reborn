//! 消息内容解析
//!
//! 根据 msg_type 解析原始 content 为结构化 [`MsgContent`]。
//! 支持 16+ 种消息类型, 使用 quick-xml 解析 XML 元数据。

use crate::types::{AppKind, ChatRecordItem, MsgContent};

/// HTML 实体解码
pub fn decode_html_entities(s: &str) -> String {
    // 先处理命名实体
    let stage1 = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");
    // 再处理数字实体 (&#xHEX; / &#DEC;)
    decode_numeric_entities(&stage1)
}

/// 解码 &#xHEX; 与 &#DEC; 数字实体 (无 regex 依赖)
fn decode_numeric_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if rest.starts_with("&#x") || rest.starts_with("&#X") {
            if let Some(semi) = rest.find(';') {
                let hex = &rest[3..semi];
                if let Ok(code) = u32::from_str_radix(hex, 16) {
                    if let Some(c) = char::from_u32(code) {
                        out.push(c);
                        i += semi + 1;
                        continue;
                    }
                }
            }
        } else if rest.starts_with("&#") {
            if let Some(semi) = rest.find(';') {
                let dec = &rest[2..semi];
                if let Ok(code) = dec.parse::<u32>() {
                    if let Some(c) = char::from_u32(code) {
                        out.push(c);
                        i += semi + 1;
                        continue;
                    }
                }
            }
        }
        // 否则按字符推进 (UTF-8 安全)
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// 判断字符串是否像 wxid (wxid_ 前缀, 或 ^wx[a-z0-9_-]{4,}$)
pub fn looks_like_wxid(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let trimmed = s.trim().to_lowercase();
    if trimmed.starts_with("wxid_") {
        return true;
    }
    if !trimmed.starts_with("wx") || trimmed.len() < 6 {
        return false;
    }
    let rest = &trimmed[2..];
    if rest.len() < 4 {
        return false;
    }
    rest.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// 清理引用内容中的 wxid 与多余分隔符/空白
pub fn sanitize_quoted_content(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    // 1. 移除所有 wxid_[A-Za-z0-9_-]{3,} 模式
    let mut stripped = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if rest.starts_with("wxid_") {
            // 统计后续合法字符数量
            let mut j = i + "wxid_".len();
            let mut count = 0usize;
            for c in s[j..].chars() {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    j += c.len_utf8();
                    count += 1;
                } else {
                    break;
                }
            }
            if count >= 3 {
                i = j;
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        stripped.push(ch);
        i += ch.len_utf8();
    }

    // 2. 去掉开头的分隔符
    let trim_pred = |c: char| c.is_whitespace() || c == ':' || c == '：' || c == '-';
    let mut result = stripped.trim_start_matches(trim_pred).to_string();

    // 3. 折叠重复冒号分隔符
    while result.contains("::") || result.contains("：：") {
        result = result.replace("::", ":").replace("：：", "：");
    }
    result = result.trim_start_matches(trim_pred).to_string();

    // 4. 标准化空白
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 从 msg_type 和 content 中解析引用消息
///
/// 支持两种格式：
/// 1. msg_type=49 且 <type>57</type> — 标准 appmsg 内嵌 refermsg
/// 2. msg_type 高位如 244813135921 — content 直接是 <refermsg>...</refermsg>
pub fn parse_quote_message(_msg_type: i64, content: &str) -> Option<MsgContent> {
    // 定位 <refermsg>...</refermsg> 段落
    let refer_start = content.find("<refermsg>")?;
    let refer_end = content.find("</refermsg>")?;
    if refer_end <= refer_start {
        return None;
    }
    let refer_end_full = refer_end + "</refermsg>".len();
    let refer_xml = &content[refer_start..refer_end_full];

    // displayname (过滤 wxid)
    let mut display_name = extract_xml_text(refer_xml, "displayname");
    if let Some(ref name) = display_name {
        if looks_like_wxid(name) {
            display_name = None;
        }
    }

    // refer content (需 HTML 实体解码)
    // 注意: <content> 内可能包含嵌套 XML (如图片/img/emoji 标签),
    // extract_xml_text 只提取纯文本, 因此需要原始字符串匹配作为 fallback
    let refer_content_raw = extract_xml_text(refer_xml, "content").unwrap_or_else(|| {
        raw_tag_content(refer_xml, "content").unwrap_or_default()
    });
    let refer_content = decode_html_entities(&refer_content_raw);

    // refer type
    let refer_type = extract_xml_text(refer_xml, "type").unwrap_or_default();

    // 引用消息必有附言，且附言只会出现在 refermsg 之前。
    // 限定范围可避免误取被引用链接自身的标题。
    let comment = extract_xml_text(&content[..refer_start], "title")
        .map(|t| decode_html_entities(&t))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())?;

    // 根据 refer_type 渲染 quoted_content, 并按需提取 md5 / cdn
    let (quoted_content, image_md5, emoji_md5, emoji_cdn_url) = match refer_type.as_str() {
        "1" => (sanitize_quoted_content(&refer_content), None, None, None),
        "3" => {
            let md5 = extract_xml_text(&refer_content, "md5")
                .or_else(|| extract_xml_attr(&refer_content, "img", "md5"))
                .map(|s| s.to_lowercase());
            ("[图片]".to_string(), md5, None, None)
        }
        "34" => ("[语音]".to_string(), None, None, None),
        "43" => ("[视频]".to_string(), None, None, None),
        "47" => {
            let cdn = extract_xml_attr(&refer_content, "emoji", "cdnurl");
            let md5 = extract_xml_attr(&refer_content, "emoji", "md5").map(|s| s.to_lowercase());
            ("[动画表情]".to_string(), None, md5, cdn)
        }
        "49" => {
            let app_title = extract_xml_text(&refer_content, "title");
            (
                app_title.unwrap_or_else(|| "[链接]".to_string()),
                None,
                None,
                None,
            )
        }
        "42" => ("[名片]".to_string(), None, None, None),
        "48" => ("[位置]".to_string(), None, None, None),
        _ => {
            if refer_content.is_empty() || refer_content.contains("wxid_") {
                ("[消息]".to_string(), None, None, None)
            } else {
                (sanitize_quoted_content(&refer_content), None, None, None)
            }
        }
    };

    Some(MsgContent::Quote {
        quoted_content,
        quoted_sender: display_name,
        image_md5,
        emoji_md5,
        emoji_cdn_url,
        comment,
    })
}

/// 根据 msg_type 解析原始 content 为结构化 MsgContent
/// content 已经过 Zstd 解压 (如果需要), 应为 XML 或纯文本
pub fn parse_msg_content(msg_type: i64, content: &str) -> MsgContent {
    // 特殊类型: msg_type=244813135921 — content 直接是 <refermsg>...</refermsg>
    // 该值低 16 位虽为 49, 但 refermsg 不在 <appmsg> 内, 需优先处理
    if msg_type == 244813135921 {
        if let Some(quote) = parse_quote_message(msg_type, content) {
            return quote;
        }
    }

    // 微信 msg_type 高位是标志位 (如 0x600000021), 实际类型在低 16 位
    let base_type = (msg_type & 0xFFFF) as i32;
    match base_type {
        1 => MsgContent::Text { text: decode_html_entities(content) },
        3 => parse_image(content),
        34 => parse_voice(content),
        42 => parse_contact_card(content),
        43 => parse_video(content),
        47 => parse_emoji(content),
        48 => parse_location(content),
        49 => {
            // 先尝试引用消息 (type=57); 否则按普通 appmsg 解析
            if let Some(quote) = parse_quote_message(msg_type, content) {
                return quote;
            }
            parse_app(content)
        }
        10000 | 10002 => MsgContent::System { text: decode_html_entities(content) },
        _ => MsgContent::Unknown { raw: content.to_string(), msg_type },
    }
}

/// 图片消息: 从 XML 中提取 CDN URL + 元数据
fn parse_image(content: &str) -> MsgContent {
    let path = extract_xml_attr(content, "img", "cdnmidimgurl")
        .or_else(|| extract_xml_attr(content, "img", "cdnbigimgurl"));
    let md5 = extract_xml_attr(content, "img", "md5");
    let length = extract_xml_attr(content, "img", "length")
        .and_then(|v| v.parse::<u64>().ok());
    let width = extract_xml_attr(content, "img", "cdnmidwidth")
        .or_else(|| extract_xml_attr(content, "img", "cdnthumbwidth"))
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v > 0);
    let height = extract_xml_attr(content, "img", "cdnmidheight")
        .or_else(|| extract_xml_attr(content, "img", "cdnthumbheight"))
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v > 0);
    MsgContent::Image { path, md5, length, width, height }
}

/// 语音消息: 提取时长 + CDN URL + AES 密钥
fn parse_voice(content: &str) -> MsgContent {
    let duration_ms = extract_xml_attr(content, "voicemsg", "voicelength")
        .or_else(|| extract_xml_attr(content, "voicemsg", "voicelen"))
        .or_else(|| extract_xml_attr(content, "voicemsg", "length"))
        .and_then(|v| v.parse::<u32>().ok());
    let voice_url = extract_xml_attr(content, "voicemsg", "voiceurl");
    let aeskey = extract_xml_attr(content, "voicemsg", "aeskey");
    MsgContent::Voice { duration_ms, voice_url, aeskey }
}

/// 名片消息 (msg_type=42): 提取昵称、wxid、头像
fn parse_contact_card(content: &str) -> MsgContent {
    let nickname = extract_xml_attr(content, "msg", "nickname");
    let username = extract_xml_attr(content, "msg", "username");
    let avatar_url = extract_xml_attr(content, "msg", "smallheadimgurl");
    MsgContent::ContactCard { nickname, username, avatar_url }
}

/// 视频消息: 提取缩略图 + 视频 CDN + 元数据
fn parse_video(content: &str) -> MsgContent {
    let thumb_path = extract_xml_attr(content, "videomsg", "cdnthumburl");
    let cdn_video_url = extract_xml_attr(content, "videomsg", "cdnvideourl");
    let aeskey = extract_xml_attr(content, "videomsg", "aeskey");
    let length = extract_xml_attr(content, "videomsg", "length")
        .and_then(|v| v.parse::<u64>().ok());
    let play_length = extract_xml_attr(content, "videomsg", "playlength")
        .and_then(|v| v.parse::<u32>().ok());
    let width = extract_xml_attr(content, "videomsg", "cdnthumbwidth")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v > 0);
    let height = extract_xml_attr(content, "videomsg", "cdnthumbheight")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v > 0);
    MsgContent::Video { thumb_path, cdn_video_url, aeskey, length, play_length, width, height }
}

/// 表情消息: 提取 cdnurl
fn parse_emoji(content: &str) -> MsgContent {
    let url = extract_xml_attr(content, "emoji", "cdnurl");
    MsgContent::Emoji { url }
}

/// 位置消息 (msg_type=48): 提取坐标、名称、地址
fn parse_location(content: &str) -> MsgContent {
    let x = extract_xml_attr(content, "location", "x")
        .and_then(|v| v.parse::<f64>().ok());
    let y = extract_xml_attr(content, "location", "y")
        .and_then(|v| v.parse::<f64>().ok());
    let scale = extract_xml_attr(content, "location", "scale")
        .and_then(|v| v.parse::<u32>().ok());
    let label = extract_xml_attr(content, "location", "label");
    let poiname = extract_xml_attr(content, "location", "poiname");
    MsgContent::Location { x, y, scale, label, poiname }
}

/// App/文件消息 (msg_type=49): 解析 appmsg XML
///
/// 文件 (6/74) 和引用 (57) 使用独立 MsgContent variant，其他子类型由
/// AppKind 分类；聊天记录 (19) 同时解析内嵌的 dataitem 列表。
fn parse_app(content: &str) -> MsgContent {
    let title = extract_xml_text(content, "title");
    let desc = extract_xml_text(content, "des");
    let url = extract_xml_text(content, "url");
    let app_type = extract_xml_text(content, "type")
        .and_then(|t| t.parse::<i32>().ok());

    // 引用消息 (type=57): 委托给 parse_quote_message
    if app_type == Some(57) {
        if let Some(quote) = parse_quote_message(49, content) {
            return quote;
        }
        // 解析失败则继续按普通 appmsg 处理 (容错)
    }

    let is_file = matches!(app_type, Some(6) | Some(74))
        || (content.contains("<appattach>")
            && content.contains("<fileext>")
            && extract_xml_text(content, "totallen")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
                > 0);

    if is_file {
        let file_size = extract_xml_text(content, "totallen")
            .or_else(|| extract_xml_text(content, "filesize"))
            .and_then(|v| v.parse::<u64>().ok());
        let file_ext = extract_xml_text(content, "fileext");
        let md5 = extract_xml_text(content, "md5");
        return MsgContent::File { title, file_size, file_ext, md5 };
    }

    // subtype=19 的聊天记录把 dataitem XML 包在 recorditem CDATA 中。
    let record_item_xml = if app_type == Some(19) {
        extract_record_item_xml(content)
    } else {
        None
    };
    let record_items = record_item_xml
        .as_deref()
        .map(parse_chat_record_items)
        .unwrap_or_default();

    MsgContent::App {
        title,
        desc,
        url,
        app_type,
        kind: AppKind::from_app_type(app_type),
        record_item_xml,
        record_items,
    }
}

/// 从 XML 中提取指定元素的属性值 (如 <img cdnmidimgurl="..."/>)
pub fn extract_xml_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == tag.as_bytes() {
                    for a in e.attributes().flatten() {
                        if a.key.as_ref() == attr.as_bytes() {
                            return String::from_utf8(a.value.to_vec())
                                .ok()
                                .map(|value| decode_html_entities(&value));
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

/// 从 XML 中提取指定元素内的原始字符串 (支持嵌套 XML 标签)
/// 用于提取 <content>...</content> 内包含子元素的情况
fn raw_tag_content(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);

    let start = xml.find(&open)?;
    // 跳过 <tag ...> 到 > 的位置
    let inner_start = xml[start..].find('>')? + 1 + start;

    let end = xml[inner_start..].find(&close)? + inner_start;
    Some(xml[inner_start..end].to_string())
}

/// 解包聊天记录 `<recorditem>` 中的 CDATA，返回内层 XML。
///
/// CDATA 内的 `<dataitem>` 不会被外层 XML reader 当作节点解析，因此必须
/// 先取出 CDATA 文本，才能在 A5 中继续解析每一条记录。只接受
/// `recorditem` 的直接 CDATA 子节点，避免误取其他位置的 CDATA。
fn extract_record_item_xml(xml: &str) -> Option<String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut recorditem_depth = None;
    let mut inner = String::new();
    let mut saw_cdata = false;
    let mut recorditem_complete = false;
    let mut extracted = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if !recorditem_complete && recorditem_depth == Some(depth) {
                    // recorditem 的直接内容应当只有 CDATA（和空白）。
                    return None;
                }
                depth += 1;
                if !recorditem_complete
                    && recorditem_depth.is_none()
                    && e.name().as_ref() == b"recorditem"
                {
                    recorditem_depth = Some(depth);
                }
            }
            Ok(Event::CData(ref e)) if !recorditem_complete && recorditem_depth == Some(depth) => {
                let part = String::from_utf8(e.to_vec()).ok()?;
                inner.push_str(&part);
                saw_cdata = true;
            }
            Ok(Event::Text(ref e)) if !recorditem_complete && recorditem_depth == Some(depth) => {
                // 允许 CDATA 前后的缩进空白，但不接受裸文本作为内层 XML。
                let text = String::from_utf8(e.to_vec()).ok()?;
                if !text.trim().is_empty() {
                    return None;
                }
            }
            Ok(Event::End(ref e)) => {
                if !recorditem_complete
                    && recorditem_depth == Some(depth)
                    && e.name().as_ref() == b"recorditem"
                {
                    let result = inner.trim();
                    extracted = if saw_cdata && !result.is_empty() {
                        Some(result.to_string())
                    } else {
                        None
                    };
                    recorditem_complete = true;
                    recorditem_depth = None;
                }
                depth = depth.checked_sub(1)?;
            }
            Ok(Event::Empty(ref e)) => {
                if !recorditem_complete && recorditem_depth == Some(depth) {
                    return None;
                }
                if !recorditem_complete && recorditem_depth.is_none() {
                    // `<recorditem/>` contains no CDATA and is therefore not useful.
                    if e.name().as_ref() == b"recorditem" {
                        recorditem_complete = true;
                    }
                }
            }
            Ok(Event::Eof) => return extracted,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

/// 解析 `<recorditem>` 内层 XML 中的全部 `<dataitem>`。
///
/// 每个 dataitem 独立解析；单条字段缺失或数值非法时保留其余字段，避免一条
/// 异常记录导致整份合并转发不可用。
fn parse_chat_record_items(xml: &str) -> Vec<ChatRecordItem> {
    let mut items = Vec::new();
    let mut offset = 0usize;

    while let Some(relative_start) = find_dataitem_start(&xml[offset..]) {
        let start = offset + relative_start;
        let Some(relative_open_end) = xml[start..].find('>') else {
            break;
        };
        let open_end = start + relative_open_end;
        let opening = &xml[start..=open_end];

        if opening.trim_end().ends_with("/>") {
            items.push(parse_chat_record_item(opening));
            offset = open_end + 1;
            continue;
        }

        let body_start = open_end + 1;
        let close = xml[body_start..]
            .find("</dataitem>")
            .map(|position| body_start + position);
        let next = find_dataitem_start(&xml[body_start..])
            .map(|position| body_start + position);

        // 未闭合的坏条目不能吞掉后续合法条目。
        if let Some(next) = next {
            if close.is_none() || next < close.unwrap() {
                offset = next;
                continue;
            }
        }

        let Some(close) = close else {
            break;
        };
        let end = close + "</dataitem>".len();
        items.push(parse_chat_record_item(&xml[start..end]));
        offset = end;
    }

    items
}

fn find_dataitem_start(xml: &str) -> Option<usize> {
    let mut offset = 0usize;
    while let Some(relative) = xml[offset..].find("<dataitem") {
        let start = offset + relative;
        let after_name = start + "<dataitem".len();
        match xml.as_bytes().get(after_name) {
            Some(b'>') | Some(b'/') | Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n') => {
                return Some(start);
            }
            Some(_) => offset = after_name,
            None => return None,
        }
    }
    None
}

fn parse_chat_record_item(xml: &str) -> ChatRecordItem {
    ChatRecordItem {
        datatype: extract_xml_attr(xml, "dataitem", "datatype")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0),
        data_desc: record_text(xml, "datadesc"),
        data_title: record_text(xml, "datatitle"),
        source_name: record_text(xml, "sourcename"),
        source_time: record_text(xml, "sourcetime"),
        source_head_url: record_text(xml, "sourceheadurl"),
        file_ext: record_text(xml, "fileext"),
        data_size: record_number(xml, "datasize"),
        message_uuid: record_text(xml, "messageuuid"),
        data_url: record_text(xml, "dataurl"),
        thumb_url: record_text(xml, "datathumburl")
            .or_else(|| record_text(xml, "thumburl"))
            .or_else(|| record_text(xml, "thumbheadurl")),
        cdn_url: record_text(xml, "datacdnurl")
            .or_else(|| record_text(xml, "cdnurl")),
        aes_key: record_text(xml, "aeskey")
            .or_else(|| record_text(xml, "qaeskey")),
        md5: record_text(xml, "md5")
            .or_else(|| record_text(xml, "datamd5")),
        image_height: record_number(xml, "imgheight"),
        image_width: record_number(xml, "imgwidth"),
        duration: record_number(xml, "duration"),
    }
}

fn record_text(xml: &str, tag: &str) -> Option<String> {
    let raw = raw_tag_content(xml, tag)?;
    let trimmed = raw.trim();
    let value = if let Some(value) = trimmed.strip_prefix("<![CDATA[") {
        value.strip_suffix("]]>").unwrap_or(value)
    } else {
        trimmed
    };
    let decoded = decode_html_entities(value.trim());
    let decoded = decoded.trim();
    (!decoded.is_empty()).then(|| decoded.to_string())
}

fn record_number<T>(xml: &str, tag: &str) -> Option<T>
where
    T: std::str::FromStr,
{
    record_text(xml, tag)?.parse::<T>().ok()
}

/// 从 XML 中提取指定元素的文本内容 (如 <title>标题</title>)
pub fn extract_xml_text(xml: &str, tag: &str) -> Option<String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut in_tag = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if e.name().as_ref() == tag.as_bytes() {
                    in_tag = true;
                }
            }
            Ok(Event::Text(ref e)) if in_tag => {
                return String::from_utf8(e.to_vec())
                    .ok()
                    .map(|text| decode_html_entities(&text));
            }
            Ok(Event::CData(ref e)) if in_tag => {
                return String::from_utf8(e.to_vec()).ok();
            }
            Ok(Event::End(ref e)) => {
                if e.name().as_ref() == tag.as_bytes() {
                    in_tag = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

// =====================================================================
// Protobuf 手工解析 (chat_room ext_buffer)
// =====================================================================

/// 从 chat_room.ext_buffer (protobuf) 中提取所有成员 wxid
///
/// 格式 (基于实测 hex dump):
///
/// 重复 {
/// 0x0a <len> { // 外层 field 1 (嵌套消息)
/// 0x0a <len> <wxid_bytes> // 内层 field 1 = 成员 wxid
/// 0x18 0x01 // 内层 field 3 = 标志
/// 0x22 <len> <wxid_bytes> // 内层 field 4 = 邀请人 wxid (可选)
/// }
/// }
/// 0x18 <varint> // 尾部 field 3 = 群版本号
/// 0x20 <varint> // 尾部 field 4 = 群版本号
///

pub fn parse_ext_buffer_wxids(buf: &[u8]) -> Vec<String> {
    let mut wxids = Vec::new();
    let mut pos = 0;
    while pos < buf.len() {
        let tag = buf[pos];
        pos += 1;
        let wire_type = tag & 0x07;
        match wire_type {
            2 => {
                let (len, consumed) = read_varint(&buf[pos..]);
                pos += consumed;
                if pos + len > buf.len() {
                    break;
                }
                let field_num = tag >> 3;
                let data = &buf[pos..pos + len];
                if field_num == 1 {
                    if let Some(wxid) = extract_inner_field1_string(data) {
                        if !wxid.is_empty() && !wxids.contains(&wxid) {
                            wxids.push(wxid);
                        }
                    }
                }
                pos += len;
            }
            0 => {
                let (_, consumed) = read_varint(&buf[pos..]);
                pos += consumed;
            }
            5 => {
                pos += 4;
            }
            1 => {
                pos += 8;
            }
            _ => break,
        }
    }
    wxids
}

/// 从嵌套 protobuf 消息中提取 field 1 的字符串值
fn extract_inner_field1_string(buf: &[u8]) -> Option<String> {
    let mut pos = 0;
    while pos < buf.len() {
        let tag = buf[pos];
        pos += 1;
        let wire_type = tag & 0x07;
        let field_num = tag >> 3;
        match wire_type {
            2 => {
                let (len, consumed) = read_varint(&buf[pos..]);
                pos += consumed;
                if pos + len > buf.len() {
                    return None;
                }
                if field_num == 1 {
                    return String::from_utf8(buf[pos..pos + len].to_vec()).ok();
                }
                pos += len;
            }
            0 => {
                let (_, consumed) = read_varint(&buf[pos..]);
                pos += consumed;
            }
            5 => {
                pos += 4;
            }
            1 => {
                pos += 8;
            }
            _ => return None,
        }
    }
    None
}

/// 读取 protobuf varint, 返回 (值, 消耗的字节数)
fn read_varint(buf: &[u8]) -> (usize, usize) {
    let mut result: usize = 0;
    let mut shift = 0;
    for (i, &byte) in buf.iter().enumerate() {
        result |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return (result, i + 1);
        }
        shift += 7;
        if shift >= 64 {
            break;
        }
    }
    (result, buf.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text() {
        let mc = parse_msg_content(1, "hello world");
        assert!(matches!(mc, MsgContent::Text { ref text } if text == "hello world"));
    }

    #[test]
    fn test_parse_text_decodes_html_entities() {
        let mc = parse_msg_content(
            1,
            "A&amp;B&lt;tag&gt;&quot;Q&quot;&apos;S&apos;&nbsp;&#x4e2d;&#25991;",
        );
        assert!(matches!(mc, MsgContent::Text { ref text } if text == "A&B<tag>\"Q\"'S' 中文"));
    }

    #[test]
    fn test_parse_system() {
        let mc = parse_msg_content(10000, "撤回了一条消息");
        assert!(matches!(mc, MsgContent::System { ref text } if text == "撤回了一条消息"));
    }

    #[test]
    fn test_parse_system_decodes_html_entities() {
        let mc = parse_msg_content(10000, "Tom &amp; Jerry &#x1f44d;");
        assert!(matches!(mc, MsgContent::System { ref text } if text == "Tom & Jerry 👍"));
    }

    #[test]
    fn test_parse_image() {
        let xml = r#"<msg><img cdnmidimgurl="http://example.com/img.jpg" md5="abc123" length="1024" cdnmidwidth="800" cdnmidheight="600"/></msg>"#;
        let mc = parse_msg_content(3, xml);
        match mc {
            MsgContent::Image { path, md5, length, width, height } => {
                assert_eq!(path.as_deref(), Some("http://example.com/img.jpg"));
                assert_eq!(md5.as_deref(), Some("abc123"));
                assert_eq!(length, Some(1024));
                assert_eq!(width, Some(800));
                assert_eq!(height, Some(600));
            }
            _ => panic!("expected Image"),
        }
    }

    #[test]
    fn test_parse_emoji() {
        let xml = r#"<msg><emoji cdnurl="http://example.com/emoji.gif"/></msg>"#;
        let mc = parse_msg_content(47, xml);
        assert!(matches!(mc, MsgContent::Emoji { ref url } if url.as_deref() == Some("http://example.com/emoji.gif")));
    }

    #[test]
    fn test_parse_location() {
        let xml = r#"<msg><location x="39.9042" y="116.4074" scale="16" label="北京市" poiname="天安门"/></msg>"#;
        let mc = parse_msg_content(48, xml);
        match mc {
            MsgContent::Location { x, y, scale, label, poiname } => {
                assert_eq!(x, Some(39.9042));
                assert_eq!(y, Some(116.4074));
                assert_eq!(scale, Some(16));
                assert_eq!(label.as_deref(), Some("北京市"));
                assert_eq!(poiname.as_deref(), Some("天安门"));
            }
            _ => panic!("expected Location"),
        }
    }

    #[test]
    fn test_parse_app_link() {
        let xml = r#"<msg><appmsg><title>测试链接</title><des>描述内容</des><url>http://example.com</url><type>5</type></appmsg></msg>"#;
        let mc = parse_msg_content(49, xml);
        match mc {
            MsgContent::App {
                title,
                desc,
                url,
                app_type,
                kind,
                record_item_xml,
                record_items,
            } => {
                assert_eq!(title.as_deref(), Some("测试链接"));
                assert_eq!(desc.as_deref(), Some("描述内容"));
                assert_eq!(url.as_deref(), Some("http://example.com"));
                assert_eq!(app_type, Some(5));
                assert_eq!(kind, AppKind::Link);
                assert_eq!(record_item_xml, None);
                assert!(record_items.is_empty());
            }
            _ => panic!("expected App"),
        }
    }

    #[test]
    fn test_parse_app_decodes_html_entities() {
        let xml = r#"<msg><appmsg><title>A&amp;B&nbsp;&#x4e2d;&#25991;</title><des>&lt;描述&gt;</des><url>https://example.com/?a=1&amp;b=2</url><type>5</type></appmsg></msg>"#;
        let mc = parse_msg_content(49, xml);
        match mc {
            MsgContent::App { title, desc, url, .. } => {
                assert_eq!(title.as_deref(), Some("A&B 中文"));
                assert_eq!(desc.as_deref(), Some("<描述>"));
                assert_eq!(url.as_deref(), Some("https://example.com/?a=1&b=2"));
            }
            _ => panic!("expected App"),
        }
    }

    #[test]
    fn test_parse_app_subtypes() {
        let cases = [
            (3, AppKind::Music, "音乐"),
            (4, AppKind::Link, "链接"),
            (5, AppKind::Link, "链接"),
            (19, AppKind::ChatRecord, "聊天记录"),
            (33, AppKind::MiniProgram, "小程序"),
            (36, AppKind::MiniProgram, "小程序"),
            (49, AppKind::Link, "链接"),
            (62, AppKind::Pat, "拍一拍"),
            (87, AppKind::Announcement, "群公告"),
            (115, AppKind::Gift, "微信礼物"),
            (2000, AppKind::Transfer, "转账"),
            (2001, AppKind::RedPacket, "红包"),
        ];

        for (app_type, expected_kind, expected_label) in cases {
            let xml = format!(
                "<msg><appmsg><title>测试标题</title><type>{app_type}</type></appmsg></msg>"
            );
            let mc = parse_msg_content(49, &xml);
            match &mc {
                MsgContent::App { app_type: parsed_type, kind, .. } => {
                    assert_eq!(*parsed_type, Some(app_type));
                    assert_eq!(*kind, expected_kind);
                    assert_eq!(mc.type_label(), expected_label);
                    assert_eq!(mc.preview(100), format!("[{expected_label}] 测试标题"));
                }
                _ => panic!("expected App for subtype {app_type}"),
            }
        }
    }

    #[test]
    fn test_parse_app_unknown_subtype() {
        let xml = r#"<msg><appmsg><title>未知应用消息</title><type>999</type></appmsg></msg>"#;
        let mc = parse_msg_content(49, xml);
        match mc {
            MsgContent::App { app_type, kind, .. } => {
                assert_eq!(app_type, Some(999));
                assert_eq!(kind, AppKind::Unknown);
            }
            _ => panic!("expected App"),
        }
    }

    #[test]
    fn test_app_kind_serializes_as_snake_case() {
        let xml = r#"<msg><appmsg><title>歌曲</title><type>3</type></appmsg></msg>"#;
        let value = serde_json::to_value(parse_msg_content(49, xml)).expect("serialize MsgContent");
        assert_eq!(value["type"], "App");
        assert_eq!(value["data"]["app_type"], 3);
        assert_eq!(value["data"]["kind"], "music");
        assert!(!value["data"].as_object().unwrap().contains_key("record_item_xml"));
        assert!(!value["data"].as_object().unwrap().contains_key("record_items"));
    }

    #[test]
    fn test_parse_chat_record_extracts_record_item_cdata() {
        let xml = r#"<msg><appmsg><title>群聊的聊天记录</title><type>19</type><recorditem name="history">
<![CDATA[
<recordinfo>
  <dataitem datatype="1"><sourcename>张三</sourcename><datadesc>A&amp;B</datadesc></dataitem>
  <dataitem datatype="3"><sourcename>李四</sourcename><datadesc>图片</datadesc></dataitem>
</recordinfo>
]]>
</recorditem></appmsg></msg>"#;

        let mc = parse_msg_content(49, xml);
        match mc {
            MsgContent::App { kind, app_type, record_item_xml, .. } => {
                assert_eq!(kind, AppKind::ChatRecord);
                assert_eq!(app_type, Some(19));
                let inner = record_item_xml.expect("recorditem CDATA should be extracted");
                assert!(inner.starts_with("<recordinfo>"));
                assert!(inner.contains("<dataitem datatype=\"1\">"));
                assert!(inner.contains("<dataitem datatype=\"3\">"));
                assert!(inner.contains("A&amp;B"));
                assert!(!inner.contains("<![CDATA["));
                assert!(!inner.contains("]]>"));
            }
            _ => panic!("expected chat-record App"),
        }
    }

    #[test]
    fn test_parse_chat_record_serializes_unwrapped_xml() {
        let xml = r#"<msg><appmsg><title>聊天记录</title><type>19</type><recorditem><![CDATA[<recordinfo><dataitem datatype="1"/></recordinfo>]]></recorditem></appmsg></msg>"#;
        let value = serde_json::to_value(parse_msg_content(49, xml)).expect("serialize MsgContent");
        assert_eq!(value["data"]["kind"], "chat_record");
        let record_xml = value["data"]["record_item_xml"].as_str().unwrap();
        assert_eq!(record_xml, "<recordinfo><dataitem datatype=\"1\"/></recordinfo>");
        assert!(!record_xml.contains("<![CDATA["));
        assert!(!record_xml.contains("]]>"));
        assert_eq!(value["data"]["record_items"][0]["datatype"], 1);
    }

    #[test]
    fn test_parse_chat_record_deep_fields_and_aliases() {
        let xml = r#"<msg><appmsg><title>完整聊天记录</title><type>19</type><recorditem><![CDATA[
<recordinfo>
  <dataitem datatype="1">
    <datadesc>Hello&#x20;&amp;&#32;世界</datadesc>
    <datatitle>第一行&#x0A;第二行</datatitle>
    <sourcename>张三&amp;Co</sourcename>
    <sourcetime>2026-07-18 12:00:00</sourcetime>
    <sourceheadurl>https://example.com/avatar?a=1&amp;b=2</sourceheadurl>
    <fileext>jpg</fileext><datasize>123456</datasize><messageuuid>uuid-1</messageuuid>
    <dataurl>https://example.com/data?a=1&amp;b=2</dataurl>
    <datathumburl>https://example.com/thumb</datathumburl>
    <thumburl>https://example.com/ignored-thumb</thumburl>
    <datacdnurl>https://example.com/cdn</datacdnurl>
    <cdnurl>https://example.com/ignored-cdn</cdnurl>
    <aeskey>primary-key</aeskey><qaeskey>fallback-key</qaeskey>
    <md5>ABCDEF</md5><datamd5>ignored-md5</datamd5>
    <imgheight>720</imgheight><imgwidth>1280</imgwidth><duration>3210</duration>
  </dataitem>
  <dataitem datatype="3"><sourcename>李四</sourcename><datadesc>图片</datadesc></dataitem>
</recordinfo>
]]></recorditem></appmsg></msg>"#;

        let mc = parse_msg_content(49, xml);
        let MsgContent::App { record_items, .. } = mc else {
            panic!("expected chat-record App");
        };
        assert_eq!(record_items.len(), 2);

        let first = &record_items[0];
        assert_eq!(first.datatype, 1);
        assert_eq!(first.data_desc.as_deref(), Some("Hello & 世界"));
        assert_eq!(first.data_title.as_deref(), Some("第一行\n第二行"));
        assert_eq!(first.source_name.as_deref(), Some("张三&Co"));
        assert_eq!(first.source_time.as_deref(), Some("2026-07-18 12:00:00"));
        assert_eq!(first.source_head_url.as_deref(), Some("https://example.com/avatar?a=1&b=2"));
        assert_eq!(first.file_ext.as_deref(), Some("jpg"));
        assert_eq!(first.data_size, Some(123456));
        assert_eq!(first.message_uuid.as_deref(), Some("uuid-1"));
        assert_eq!(first.data_url.as_deref(), Some("https://example.com/data?a=1&b=2"));
        assert_eq!(first.thumb_url.as_deref(), Some("https://example.com/thumb"));
        assert_eq!(first.cdn_url.as_deref(), Some("https://example.com/cdn"));
        assert_eq!(first.aes_key.as_deref(), Some("primary-key"));
        assert_eq!(first.md5.as_deref(), Some("ABCDEF"));
        assert_eq!(first.image_height, Some(720));
        assert_eq!(first.image_width, Some(1280));
        assert_eq!(first.duration, Some(3210));

        assert_eq!(record_items[1].datatype, 3);
        assert_eq!(record_items[1].source_name.as_deref(), Some("李四"));
        assert_eq!(record_items[1].data_desc.as_deref(), Some("图片"));
    }

    #[test]
    fn test_parse_chat_record_fallback_aliases_and_invalid_numbers() {
        let inner = r#"<recordinfo><dataitem datatype="bad">
            <thumburl>thumb</thumburl><cdnurl>cdn</cdnurl><qaeskey>key</qaeskey>
            <datamd5>md5</datamd5><datasize>bad</datasize><duration>also-bad</duration>
        </dataitem></recordinfo>"#;
        let items = parse_chat_record_items(inner);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].datatype, 0);
        assert_eq!(items[0].thumb_url.as_deref(), Some("thumb"));
        assert_eq!(items[0].cdn_url.as_deref(), Some("cdn"));
        assert_eq!(items[0].aes_key.as_deref(), Some("key"));
        assert_eq!(items[0].md5.as_deref(), Some("md5"));
        assert_eq!(items[0].data_size, None);
        assert_eq!(items[0].duration, None);
    }

    #[test]
    fn test_chat_record_field_cdata_preserves_unescaped_url() {
        let inner = r#"<recordinfo><dataitem datatype="3">
            <sourcename><![CDATA[发送者 <测试>]]></sourcename>
            <dataurl><![CDATA[https://example.com/file?a=1&b=2]]></dataurl>
        </dataitem></recordinfo>"#;
        let items = parse_chat_record_items(inner);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source_name.as_deref(), Some("发送者 <测试>"));
        assert_eq!(
            items[0].data_url.as_deref(),
            Some("https://example.com/file?a=1&b=2")
        );
    }

    #[test]
    fn test_parse_chat_record_skips_unclosed_item_and_keeps_following_item() {
        let inner = r#"<recordinfo>
            <dataitem datatype="1"><datadesc>损坏条目
            <dataitem datatype="3"><datadesc>有效图片</datadesc></dataitem>
        </recordinfo>"#;
        let items = parse_chat_record_items(inner);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].datatype, 3);
        assert_eq!(items[0].data_desc.as_deref(), Some("有效图片"));
    }

    #[test]
    fn test_chat_record_json_contains_structured_items() {
        let xml = r#"<msg><appmsg><type>19</type><recorditem><![CDATA[
            <recordinfo><dataitem datatype="34"><sourcename>张三</sourcename><duration>2500</duration></dataitem></recordinfo>
        ]]></recorditem></appmsg></msg>"#;
        let value = serde_json::to_value(parse_msg_content(49, xml)).expect("serialize MsgContent");
        let item = &value["data"]["record_items"][0];
        assert_eq!(item["datatype"], 34);
        assert_eq!(item["source_name"], "张三");
        assert_eq!(item["duration"], 2500);
        assert!(item.get("data_url").is_none());
    }

    #[test]
    fn test_record_item_cdata_is_only_for_chat_record_subtype() {
        let xml = r#"<msg><appmsg><title>普通链接</title><type>5</type><recorditem><![CDATA[<recordinfo><dataitem/></recordinfo>]]></recorditem></appmsg></msg>"#;
        let mc = parse_msg_content(49, xml);
        match mc {
            MsgContent::App { kind, record_item_xml, .. } => {
                assert_eq!(kind, AppKind::Link);
                assert_eq!(record_item_xml, None);
            }
            _ => panic!("expected App"),
        }
    }

    #[test]
    fn test_record_item_cdata_requires_valid_boundary() {
        let without_cdata = r#"<msg><appmsg><type>19</type><recorditem><dataitem/></recorditem></appmsg></msg>"#;
        assert!(extract_record_item_xml(without_cdata).is_none());

        let empty_cdata = r#"<msg><appmsg><type>19</type><recorditem><![CDATA[   ]]></recorditem></appmsg></msg>"#;
        assert!(extract_record_item_xml(empty_cdata).is_none());

        let malformed = r#"<msg><appmsg><type>19</type><recorditem><![CDATA[<recordinfo><dataitem></recordinfo></recorditem></appmsg></msg>"#;
        assert!(extract_record_item_xml(malformed).is_none());
    }

    #[test]
    fn test_parse_app_file() {
        let xml = r#"<msg><appmsg><title>doc.pdf</title><type>6</type><appattach><totallen>2048</totallen><fileext>pdf</fileext></appattach></appmsg></msg>"#;
        let mc = parse_msg_content(49, xml);
        match mc {
            MsgContent::File { title, file_size, file_ext, .. } => {
                assert_eq!(title.as_deref(), Some("doc.pdf"));
                assert_eq!(file_size, Some(2048));
                assert_eq!(file_ext.as_deref(), Some("pdf"));
            }
            _ => panic!("expected File"),
        }
    }

    #[test]
    fn test_parse_app_file_subtype_74() {
        let xml = r#"<msg><appmsg><title>archive.zip</title><type>74</type><appattach><totallen>4096</totallen><fileext>zip</fileext></appattach></appmsg></msg>"#;
        let mc = parse_msg_content(49, xml);
        assert!(matches!(
            mc,
            MsgContent::File {
                ref title,
                file_size: Some(4096),
                ref file_ext,
                ..
            } if title.as_deref() == Some("archive.zip") && file_ext.as_deref() == Some("zip")
        ));
    }

    #[test]
    fn test_parse_contact_card() {
        let xml = r#"<msg nickname="张三" username="wxid_zhangsan" smallheadimgurl="http://avatar.jpg"/>"#;
        let mc = parse_msg_content(42, xml);
        match mc {
            MsgContent::ContactCard { nickname, username, avatar_url } => {
                assert_eq!(nickname.as_deref(), Some("张三"));
                assert_eq!(username.as_deref(), Some("wxid_zhangsan"));
                assert_eq!(avatar_url.as_deref(), Some("http://avatar.jpg"));
            }
            _ => panic!("expected ContactCard"),
        }
    }

    #[test]
    fn test_parse_unknown_type() {
        let mc = parse_msg_content(999, "raw content");
        assert!(matches!(mc, MsgContent::Unknown { ref raw, msg_type: 999 } if raw == "raw content"));
    }

    #[test]
    fn test_parse_msg_type_high_bits() {
        // msg_type 高位是标志位, 实际类型在低 16 位
        let mc = parse_msg_content(0x600000001, "text");
        assert!(matches!(mc, MsgContent::Text { ref text } if text == "text"));
    }

    #[test]
    fn test_extract_xml_text() {
        let xml = r#"<msg><title>标题</title><des>描述</des></msg>"#;
        assert_eq!(extract_xml_text(xml, "title").as_deref(), Some("标题"));
        assert_eq!(extract_xml_text(xml, "des").as_deref(), Some("描述"));
        assert_eq!(extract_xml_text(xml, "nonexistent"), None);
    }

    #[test]
    fn test_extract_xml_attr() {
        let xml = r#"<msg><img src="url" width="800"/></msg>"#;
        assert_eq!(extract_xml_attr(xml, "img", "src").as_deref(), Some("url"));
        assert_eq!(extract_xml_attr(xml, "img", "width").as_deref(), Some("800"));
        assert_eq!(extract_xml_attr(xml, "img", "height"), None);
    }

    #[test]
    fn test_extract_xml_attr_decodes_html_entities() {
        let xml = r#"<msg><location label="A&amp;B&nbsp;&#x4e2d;&#25991;"/></msg>"#;
        assert_eq!(
            extract_xml_attr(xml, "location", "label").as_deref(),
            Some("A&B 中文")
        );
    }

    #[test]
    fn test_preview_text() {
        let mc = MsgContent::Text { text: "hello".into() };
        assert_eq!(mc.preview(10), "hello");
    }

    #[test]
    fn test_preview_truncate() {
        let long = "a".repeat(100);
        let mc = MsgContent::Text { text: long };
        let p = mc.preview(5);
        assert!(p.ends_with("..."));
        assert!(p.len() < 10);
    }

    #[test]
    fn test_preview_image() {
        let mc = MsgContent::Image { path: None, md5: None, length: None, width: None, height: None };
        assert_eq!(mc.preview(10), "[图片]");
    }

    #[test]
    fn test_preview_file_size() {
        let mc = MsgContent::File {
            title: Some("test.zip".into()),
            file_size: Some(1048576),
            file_ext: Some("zip".into()),
            md5: None
        };
        let p = mc.preview(100);
        assert!(p.contains("1.0MB"));
    }

    #[test]
    fn test_parse_ext_buffer_empty() {
        assert!(parse_ext_buffer_wxids(&[]).is_empty());
    }

    #[test]
    fn test_parse_ext_buffer_two_members() {
        // 构造两个群成员的 protobuf:
        // field 1 (tag=0x0a) length-delimited → 嵌套消息
        //   内层 field 1 (tag=0x0a) length-delimited → wxid 字符串
        //   内层 field 3 (tag=0x18) varint → 1
        let wxid1 = b"wxid_abc123";
        let wxid2 = b"wxid_def456";
        let mut inner1 = vec![0x0a, wxid1.len() as u8];
        inner1.extend_from_slice(wxid1);
        inner1.extend_from_slice(&[0x18, 0x01]);
        let mut inner2 = vec![0x0a, wxid2.len() as u8];
        inner2.extend_from_slice(wxid2);
        inner2.extend_from_slice(&[0x18, 0x01]);
        let mut buf = vec![0x0a, inner1.len() as u8];
        buf.extend_from_slice(&inner1);
        buf.push(0x0a);
        buf.push(inner2.len() as u8);
        buf.extend_from_slice(&inner2);
        let wxids = parse_ext_buffer_wxids(&buf);
        assert_eq!(wxids.len(), 2);
        assert_eq!(wxids[0], "wxid_abc123");
        assert_eq!(wxids[1], "wxid_def456");
    }

    #[test]
    fn test_read_varint_simple() {
        assert_eq!(read_varint(&[0x01]), (1, 1));
        assert_eq!(read_varint(&[0x7f]), (127, 1));
        assert_eq!(read_varint(&[0x80, 0x01]), (128, 2));
    }

    #[test]
    fn test_parse_quote_text() {
        let xml = r#"<appmsg><type>57</type><title>看这条</title><refermsg><displayname>张三</displayname><content>你好</content><type>1</type><svrid>123</svrid></refermsg></appmsg>"#;
        let mc = parse_msg_content(49, xml);
        match mc {
            MsgContent::Quote {
                quoted_content,
                quoted_sender,
                comment,
                image_md5,
                emoji_md5,
                emoji_cdn_url,
            } => {
                assert_eq!(quoted_content, "你好");
                assert_eq!(quoted_sender.as_deref(), Some("张三"));
                assert_eq!(comment, "看这条");
                assert!(image_md5.is_none());
                assert!(emoji_md5.is_none());
                assert!(emoji_cdn_url.is_none());
            }
            _ => panic!("expected Quote"),
        }
    }

    #[test]
    fn test_parse_quote_image() {
        let xml = r#"<appmsg><type>57</type><title>看图</title><refermsg><displayname>李四</displayname><content><img md5="ABCDEF1234567890"/></content><type>3</type></refermsg></appmsg>"#;
        let mc = parse_quote_message(49, xml).expect("should parse");
        match mc {
            MsgContent::Quote { quoted_content, image_md5, .. } => {
                assert_eq!(quoted_content, "[图片]");
                assert_eq!(image_md5.as_deref(), Some("abcdef1234567890"));
            }
            _ => panic!("expected Quote"),
        }
    }

    #[test]
    fn test_parse_quote_emoji() {
        let xml = r#"<appmsg><type>57</type><title>这个表情</title><refermsg><displayname>王五</displayname><content><emoji cdnurl="http://e.com/x.gif" md5="DEADBEEF"/></content><type>47</type></refermsg></appmsg>"#;
        let mc = parse_quote_message(49, xml).expect("should parse");
        match mc {
            MsgContent::Quote { quoted_content, emoji_md5, emoji_cdn_url, .. } => {
                assert_eq!(quoted_content, "[动画表情]");
                assert_eq!(emoji_md5.as_deref(), Some("deadbeef"));
                assert_eq!(emoji_cdn_url.as_deref(), Some("http://e.com/x.gif"));
            }
            _ => panic!("expected Quote"),
        }
    }

    #[test]
    fn test_parse_quote_without_outer_comment_returns_none() {
        let xml = r#"<refermsg><displayname>赵六</displayname><content>测试消息</content><type>1</type></refermsg>"#;
        assert!(parse_quote_message(244813135921, xml).is_none());
    }

    #[test]
    fn test_parse_quote_special_type_with_outer_comment() {
        let xml = r#"<appmsg><title>高位类型附言</title><refermsg><displayname>赵六</displayname><content>测试消息</content><type>1</type></refermsg></appmsg>"#;
        let mc = parse_msg_content(244813135921, xml);
        match mc {
            MsgContent::Quote { comment, .. } => {
                assert_eq!(comment, "高位类型附言");
            }
            _ => panic!("expected Quote"),
        }
    }

    #[test]
    fn test_parse_quote_does_not_use_referenced_title_as_comment() {
        let xml = r#"<refermsg><displayname>赵六</displayname><content><![CDATA[<appmsg><title>被引用的链接</title></appmsg>]]></content><type>49</type></refermsg>"#;
        assert!(parse_quote_message(244813135921, xml).is_none());
    }

    #[test]
    fn test_parse_quote_wxid_sender_filtered() {
        let xml = r#"<appmsg><type>57</type><title>附言</title><refermsg><displayname>wxid_abc1234567</displayname><content>hello</content><type>1</type></refermsg></appmsg>"#;
        let mc = parse_quote_message(49, xml).expect("should parse");
        match mc {
            MsgContent::Quote { quoted_sender, .. } => {
                assert!(quoted_sender.is_none(), "wxid_ sender should be filtered");
            }
            _ => panic!("expected Quote"),
        }
    }

    #[test]
    fn test_parse_quote_link() {
        let xml = r#"<appmsg><type>57</type><title>转给你看</title><refermsg><displayname>钱七</displayname><content><![CDATA[<appmsg><title>某篇公众号文章</title><type>5</type></appmsg>]]></content><type>49</type></refermsg></appmsg>"#;
        let mc = parse_quote_message(49, xml).expect("should parse");
        match mc {
            MsgContent::Quote { quoted_content, comment, .. } => {
                // refer_content 是 CDATA, type=49 时取其内 <title>
                assert_eq!(quoted_content, "某篇公众号文章");
                assert_eq!(comment, "转给你看");
            }
            _ => panic!("expected Quote"),
        }
    }

    #[test]
    fn test_parse_quote_no_refermsg_returns_none() {
        let xml = r#"<appmsg><type>5</type><title>普通链接</title></appmsg>"#;
        assert!(parse_quote_message(49, xml).is_none());
    }

    #[test]
    fn test_decode_html_entities_named() {
        assert_eq!(decode_html_entities("&lt;tag&gt;"), "<tag>");
        assert_eq!(decode_html_entities("a&amp;b"), "a&b");
        assert_eq!(decode_html_entities("&quot;hi&quot;"), "\"hi\"");
        assert_eq!(decode_html_entities("&#39;ap&#39;"), "'ap'");
        assert_eq!(decode_html_entities("&nbsp;x&nbsp;"), " x ");
    }

    #[test]
    fn test_decode_html_entities_numeric() {
        assert_eq!(decode_html_entities("&#65;"), "A");
        assert_eq!(decode_html_entities("&#x41;"), "A");
        assert_eq!(decode_html_entities("&#x4e2d;&#x6587;"), "中文");
        assert_eq!(decode_html_entities("&#20013;&#25991;"), "中文");
    }

    #[test]
    fn test_decode_html_entities_preserves_invalid_entities() {
        let input = "&unknown; &#x110000; &#xD800; &#oops;";
        assert_eq!(decode_html_entities(input), input);
    }

    #[test]
    fn test_looks_like_wxid() {
        assert!(looks_like_wxid("wxid_abc123"));
        assert!(looks_like_wxid("wxid_ABC123"));
        assert!(looks_like_wxid("wxabcd1234"));
        assert!(looks_like_wxid("  wxid_x_y "));
        assert!(!looks_like_wxid("zhangsan"));
        assert!(!looks_like_wxid(""));
        assert!(!looks_like_wxid("wxab")); // 仅 2 位, 不足 4 位
    }

    #[test]
    fn test_sanitize_quoted_content_basic() {
        assert_eq!(sanitize_quoted_content("wxid_abc12345: 你好"), "你好");
    }

    #[test]
    fn test_sanitize_quoted_content_collapse_colons() {
        assert_eq!(sanitize_quoted_content("a::b"), "a:b");
    }

    #[test]
    fn test_preview_quote_with_comment() {
        let mc = MsgContent::Quote {
            quoted_content: "你好".into(),
            quoted_sender: Some("张三".into()),
            image_md5: None,
            emoji_md5: None,
            emoji_cdn_url: None,
            comment: "看这条".into(),
        };
        assert_eq!(mc.preview(100), "看这条 [引用 张三: 你好]");
    }

    #[test]
    fn test_preview_quote_truncates_comment() {
        let mc = MsgContent::Quote {
            quoted_content: "你好".into(),
            quoted_sender: None,
            image_md5: None,
            emoji_md5: None,
            emoji_cdn_url: None,
            comment: "这是一条很长的附言".into(),
        };
        assert_eq!(mc.preview(18), "这是一条很长...");
    }
}
