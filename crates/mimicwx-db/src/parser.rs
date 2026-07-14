//! 消息内容解析
//!
//! 根据 msg_type 解析原始 content 为结构化 [`MsgContent`]。
//! 支持 16+ 种消息类型, 使用 quick-xml 解析 XML 元数据。

use crate::types::MsgContent;

/// 根据 msg_type 解析原始 content 为结构化 MsgContent
/// content 已经过 Zstd 解压 (如果需要), 应为 XML 或纯文本
pub fn parse_msg_content(msg_type: i64, content: &str) -> MsgContent {
    // 微信 msg_type 高位是标志位 (如 0x600000021), 实际类型在低 16 位
    let base_type = (msg_type & 0xFFFF) as i32;
    match base_type {
        1 => MsgContent::Text { text: content.to_string() },
        3 => parse_image(content),
        34 => parse_voice(content),
        42 => parse_contact_card(content),
        43 => parse_video(content),
        47 => parse_emoji(content),
        48 => parse_location(content),
        49 => parse_app(content),
        10000 | 10002 => MsgContent::System { text: content.to_string() },
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

/// 链接/文件/小程序消息 (msg_type=49): 解析 appmsg XML
/// app_type 子类型: 3=音乐, 4=链接, 5=链接, 6=文件, 19=转发, 33/36=小程序, 2000=转账, 2001=红包
fn parse_app(content: &str) -> MsgContent {
    let title = extract_xml_text(content, "title");
    let desc = extract_xml_text(content, "des");
    let url = extract_xml_text(content, "url");
    let app_type = extract_xml_text(content, "type")
        .and_then(|t| t.parse::<i32>().ok());

    let is_file = matches!(app_type, Some(6) | Some(74))
        || (content.contains("<appattach>") && content.contains("<fileext>")
            && extract_xml_text(content, "totallen")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0) > 0);
    if is_file {
        let file_size = extract_xml_text(content, "totallen")
            .or_else(|| extract_xml_text(content, "filesize"))
            .and_then(|v| v.parse::<u64>().ok());
        let file_ext = extract_xml_text(content, "fileext");
        let md5 = extract_xml_text(content, "md5");
        return MsgContent::File { title, file_size, file_ext, md5 };
    }

    MsgContent::App { title, desc, url, app_type }
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
                            return String::from_utf8(a.value.to_vec()).ok();
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
                return e.unescape().ok().map(|s| s.to_string());
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
/// ```text
/// 重复 {
///   0x0a <len> {                    // 外层 field 1 (嵌套消息)
///     0x0a <len> <wxid_bytes>       // 内层 field 1 = 成员 wxid
///     0x18 0x01                     // 内层 field 3 = 标志
///     0x22 <len> <wxid_bytes>       // 内层 field 4 = 邀请人 wxid (可选)
///   }
/// }
/// 0x18 <varint>                     // 尾部 field 3 = 群版本号
/// 0x20 <varint>                     // 尾部 field 4 = 群版本号
/// ```
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
                if pos + len > buf.len() { break; }
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
            5 => { pos += 4; }
            1 => { pos += 8; }
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
                if pos + len > buf.len() { return None; }
                if field_num == 1 {
                    return String::from_utf8(buf[pos..pos + len].to_vec()).ok();
                }
                pos += len;
            }
            0 => {
                let (_, consumed) = read_varint(&buf[pos..]);
                pos += consumed;
            }
            5 => { pos += 4; }
            1 => { pos += 8; }
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
        if shift >= 64 { break; }
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
    fn test_parse_system() {
        let mc = parse_msg_content(10000, "撤回了一条消息");
        assert!(matches!(mc, MsgContent::System { ref text } if text == "撤回了一条消息"));
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
            MsgContent::App { title, desc, url, app_type } => {
                assert_eq!(title.as_deref(), Some("测试链接"));
                assert_eq!(desc.as_deref(), Some("描述内容"));
                assert_eq!(url.as_deref(), Some("http://example.com"));
                assert_eq!(app_type, Some(5));
            }
            _ => panic!("expected App"),
        }
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
        let mc = MsgContent::File { title: Some("test.zip".into()), file_size: Some(1048576), file_ext: Some("zip".into()), md5: None };
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
}
