//! MimicWX-Reborn: 微信自动化框架
//!
//! 架构:
//! - mimicwx-core: 基础层 (error/config/types/timing/predicates)
//! - mimicwx-atspi: AT-SPI2 底层原语 (D-Bus 通信)
//! - mimicwx-input: X11 XTEST 输入注入
//! - mimicwx-db: SQLCipher 数据库监听
//! - mimicwx-wechat: 微信业务逻辑 (控件查找/消息发送/会话管理)
//! - mimicwx-app: HTTP/WebSocket API + 交互式控制台 + 启动编排

mod api;
mod console;

use anyhow::Result;
use mimicwx_atspi::{AtSpi, registry};
use mimicwx_core::config::load_config;
use mimicwx_db::DbManager;
use mimicwx_input::InputEngine;
use mimicwx_wechat::{WeChat, WeChatStatus};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use tracing::{debug, error, info, warn};

// =====================================================================
// 自定义日志格式 (紧凑彩色终端输出)
// =====================================================================

struct CompactFormat;

impl<S, N> tracing_subscriber::fmt::FormatEvent<S, N> for CompactFormat
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> tracing_subscriber::fmt::FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let (h, m, s) = local_hms();
        write!(writer, "\x1b[2m{h:02}:{m:02}:{s:02}\x1b[0m ")?;

        match *event.metadata().level() {
            tracing::Level::ERROR => write!(writer, "\x1b[1;31mERROR\x1b[0m ")?,
            tracing::Level::WARN  => write!(writer, "\x1b[33mWARN \x1b[0m ")?,
            tracing::Level::INFO  => write!(writer, "\x1b[32mINFO \x1b[0m ")?,
            tracing::Level::DEBUG => write!(writer, "\x1b[2;36mDEBUG\x1b[0m ")?,
            tracing::Level::TRACE => write!(writer, "\x1b[2mTRACE\x1b[0m ")?,
        }

        let target = event.metadata().target();
        let tag = target
            .strip_prefix("mimicwx::")
            .unwrap_or(if target == "mimicwx" { "main" } else { target });
        write!(writer, "\x1b[1;36m{tag:<7}\x1b[0m \x1b[2m│\x1b[0m ")?;

        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

fn local_hms() -> (u32, u32, u32) {
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&t, &mut tm);
        (tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .event_format(CompactFormat)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mimicwx=info,tower_http=info".into()),
        )
        .init();

    info!(target: "mimicwx::init", "MimicWX-Reborn v{} 启动中...", env!("CARGO_PKG_VERSION"));

    // ① 加载配置文件
    let (config, config_path) = load_config();
    if !config.listen.auto.is_empty() {
        debug!(target: "mimicwx::listen", "自动监听列表: {:?}", config.listen.auto);
    }

    // ② AT-SPI2 连接 (带重试)
    let atspi = loop {
        match AtSpi::connect().await {
            Ok(a) => {
                info!(target: "mimicwx::init", "AT-SPI2 连接就绪");
                break Arc::new(a);
            }
            Err(e) => {
                warn!(target: "mimicwx::init", "AT-SPI2 连接失败: {e}, 5秒后重试");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    };

    // ③ X11 XTEST 输入引擎
    let engine = match InputEngine::new() {
        Ok(e) => {
            info!(target: "mimicwx::init", "X11 输入引擎就绪");
            Some(e)
        }
        Err(e) => {
            warn!(target: "mimicwx::init", "X11 输入引擎不可用 (发送功能受限): {e}");
            None
        }
    };

    // ④ WeChat 实例化
    let wechat = Arc::new(WeChat::new(atspi.clone(), config.timing.at_delay_ms));

    // ⑤ 等待微信就绪
    let mut attempts = 0;
    let mut login_prompted = false;
    loop {
        let status = wechat.check_status().await;
        match status {
            WeChatStatus::LoggedIn => {
                info!(target: "mimicwx::init", "微信已登录");
                break;
            }
            WeChatStatus::NotRunning if attempts < 30 => {
                debug!(target: "mimicwx::init", "等待微信启动 ({}/30)", attempts + 1);
                if attempts % 5 == 4 {
                    wechat.try_reconnect().await;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                attempts += 1;
            }
            WeChatStatus::WaitingForLogin => {
                if !login_prompted {
                    info!(target: "mimicwx::init", "请通过 noVNC (http://localhost:6080/vnc.html) 扫码登录微信");
                    info!(target: "mimicwx::key", "密钥提取已在后台运行, 登录后自动获取");
                    login_prompted = true;
                }
                attempts += 1;
                if attempts % 5 == 4 {
                    wechat.try_reconnect().await;
                }
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            _ => {
                break;
            }
        }
    }

    // ⑥ 读取数据库密钥 + 初始化 DbManager
    let key_paths = ["/home/wechat/.xwechat/wechat_key.txt", "/tmp/wechat_key.txt"];
    for i in 0..60 {
        if key_paths.iter().any(|p| std::path::Path::new(p).exists()) {
            break;
        }
        if i == 0 {
            info!(target: "mimicwx::key", "等待 extract_key.py 提取密钥...");
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    let key_path = key_paths.iter()
        .find(|p| std::path::Path::new(p).exists())
        .copied()
        .unwrap_or(key_paths[0]);

    let db_manager: Option<Arc<DbManager>> = match std::fs::read_to_string(key_path) {
        Ok(key) => {
            let key = key.trim().to_string();
            if key.len() == 96 || key.len() == 64 {
                info!(target: "mimicwx::key", "密钥已获取 ({}…{}) [{}hex]", &key[..8], &key[key.len()-8..], key.len());

                let db_dir = find_db_dir();
                match db_dir {
                    Some(dir) => {
                        let dir_for_retry = dir.clone();
                        match DbManager::new(key, dir) {
                            Ok(mgr) => {
                                let mut final_mgr = Arc::new(mgr);
                                let mark_ok = {
                                    let mut ok = false;
                                    for attempt in 0..10 {
                                        let wait = if attempt == 0 { 5 } else { 3 };
                                        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                                        match final_mgr.mark_all_read().await {
                                            Ok(()) => { ok = true; break; }
                                            Err(e) => {
                                                if attempt < 9 {
                                                    debug!(target: "mimicwx::init", "消息库未就绪 ({}/10), 3秒后重试: {}",
                                                        attempt + 1, e);
                                                } else {
                                                    warn!(target: "mimicwx::init", "标记已读失败 (已重试10次): {e}");
                                                }
                                            }
                                        }
                                    }
                                    ok
                                };
                                if let Err(e) = final_mgr.refresh_contacts().await {
                                    warn!(target: "mimicwx::init", "联系人加载失败 (可能尚无数据): {e}");
                                }
                                if !mark_ok {
                                    info!(target: "mimicwx::key", "解密失败, 可能密钥过期 — 等待新密钥...");
                                    let key_json = "/home/wechat/.xwechat/wechat_keys.json";
                                    let old_mtime = std::fs::metadata(key_json)
                                        .and_then(|m| m.modified()).ok();
                                    for _ in 0..30 {
                                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                        let new_mtime = std::fs::metadata(key_json)
                                            .and_then(|m| m.modified()).ok();
                                        if new_mtime != old_mtime && new_mtime.is_some() {
                                            info!(target: "mimicwx::key", "检测到新密钥, 重新初始化...");
                                            let new_key = key_paths.iter()
                                                .find_map(|p| std::fs::read_to_string(p).ok())
                                                .unwrap_or_default().trim().to_string();
                                            if !new_key.is_empty() {
                                                match DbManager::new(new_key, dir_for_retry.clone()) {
                                                    Ok(new_mgr) => {
                                                        let new_mgr = Arc::new(new_mgr);
                                                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                                                        let _ = new_mgr.mark_all_read().await;
                                                        let _ = new_mgr.refresh_contacts().await;
                                                        info!(target: "mimicwx::key", "新密钥解密成功, 已重新初始化");
                                                        final_mgr = new_mgr;
                                                    }
                                                    Err(e) => warn!(target: "mimicwx::key", "新密钥也失败: {e}"),
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                                Some(final_mgr)
                            }
                            Err(e) => {
                                warn!(target: "mimicwx::init", "DbManager 初始化失败: {e}");
                                None
                            }
                        }
                    }
                    None => {
                        warn!(target: "mimicwx::init", "未找到数据库目录, 监听不可用");
                        None
                    }
                }
            } else {
                warn!(target: "mimicwx::key", "密钥文件格式异常 (长度: {}), 跳过", key.len());
                None
            }
        }
        Err(_) => {
            warn!(target: "mimicwx::key", "未找到密钥文件, 解密功能不可用");
            None
        }
    };

    // ⑦ 广播通道 (WebSocket)
    let (tx, _) = tokio::sync::broadcast::channel::<String>(128);

    // ⑧ InputEngine Actor + API 服务
    let (input_tx, input_rx) = tokio::sync::mpsc::channel::<api::InputCommand>(32);

    if let Some(eng) = engine {
        api::spawn_input_actor(eng, wechat.clone(), input_rx);
    } else {
        warn!(target: "mimicwx::init", "X11 输入引擎不可用, actor 未启动");
    }

    let state = Arc::new(api::AppState {
        wechat: wechat.clone(),
        atspi: atspi.clone(),
        input_tx: input_tx.clone(),
        tx: tx.clone(),
        db: db_manager.clone(),
        api_token: config.api.token.filter(|t| !t.is_empty()),
        start_time: std::time::Instant::now(),
        config_path: config_path.clone(),
    });

    let app = api::build_router(state.clone());
    let addr = "0.0.0.0:8899";
    info!(target: "mimicwx::api", "HTTP http://{addr}  WS ws://{addr}/ws");
    info!(target: "mimicwx::api", "端点: /status /contacts /sessions /messages/new /send /chat /listen /ws");
    if state.api_token.is_some() {
        info!(target: "mimicwx::api", "认证已启用 (Bearer Token)");
    } else {
        warn!(target: "mimicwx::api", "认证未启用 (config.toml [api] token 未配置)");
    }

    let exit_code = Arc::new(AtomicI32::new(0));
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let shutdown_tx_clone = shutdown_tx.clone();
    let console_db_ref = db_manager.clone();

    // ⑧½ AT-SPI2 健康检查心跳 (每 30s, 连续 3 次异常自动重连)
    {
        let hb_atspi = atspi.clone();
        let mut hb_shutdown = shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut fail_count: u32 = 0;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = hb_shutdown.recv() => {
                        debug!(target: "mimicwx::atspi", "心跳监控停止");
                        break;
                    }
                }

                if let Some(registry) = registry() {
                    let count = hb_atspi.child_count(&registry).await;
                    if count > 0 {
                        if fail_count > 0 {
                            info!(target: "mimicwx::atspi", "连接恢复 ({count} 个应用)");
                        }
                        fail_count = 0;
                    } else {
                        fail_count += 1;
                        warn!(target: "mimicwx::atspi", "心跳异常: Registry 0 个应用 (连续 {fail_count} 次)");
                        if fail_count >= 3 {
                            warn!(target: "mimicwx::atspi", "连续 3 次异常, 尝试重连...");
                            if hb_atspi.reconnect().await {
                                fail_count = 0;
                                info!(target: "mimicwx::atspi", "重连成功");
                            } else {
                                warn!(target: "mimicwx::atspi", "重连失败, 30s 后再试");
                            }
                        }
                    }
                }
            }
        });
    }

    // ⑨ 后台数据库消息监听任务
    if let Some(db) = db_manager {
        let listen_tx = tx.clone();

        {
            let refresh_db = Arc::clone(&db);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    match refresh_db.refresh_contacts().await {
                        Ok(n) => debug!(target: "mimicwx::contact", "定时刷新完成: {n} 条"),
                        Err(e) => warn!(target: "mimicwx::contact", "定时刷新失败: {e}"),
                    }
                }
            });
        }

        let mut wal_rx = db.spawn_wal_watcher();

        tokio::spawn(async move {
            info!(target: "mimicwx::msg", "消息监听启动 (fanotify PID 过滤)");

            loop {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    wal_rx.recv(),
                ).await {
                    Ok(Ok(())) | Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                        error!(target: "mimicwx::msg", "WAL 监听通道关闭");
                        break;
                    }
                    Err(_) => {}
                }

                match db.get_new_messages().await {
                    Ok(msgs) => {
                        for m in &msgs {
                            let json = serde_json::json!({
                                "type": "db_message",
                                "chat": m.chat,
                                "chat_display": m.chat_display_name,
                                "talker": m.talker,
                                "talker_display": m.talker_display_name,
                                "content": m.content,
                                "parsed": m.parsed,
                                "msg_type": m.msg_type,
                                "create_time": m.create_time,
                                "local_id": m.local_id,
                                "is_self": m.is_self,
                                "is_at_me": m.is_at_me,
                                "at_user_list": m.at_user_list,
                            });
                            let _ = listen_tx.send(json.to_string());
                        }
                    }
                    Err(e) => {
                        debug!(target: "mimicwx::msg", "查询: {e}");
                    }
                }
            }
        });
    } else {
        warn!(target: "mimicwx::init", "数据库密钥不可用, 消息监听未启动");
    }

    // ⑩ 自动监听任务
    if !config.listen.auto.is_empty() {
        let auto_targets = config.listen.auto.clone();
        let auto_input_tx = input_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            info!(target: "mimicwx::listen", "开始自动添加监听 ({} 个目标)", auto_targets.len());

            for target in &auto_targets {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                if auto_input_tx.send(api::InputCommand::AddListen {
                    who: target.clone(),
                    reply: reply_tx,
                }).await.is_err() {
                    warn!(target: "mimicwx::listen", "actor 已停止, 无法自动添加监听");
                    break;
                }
                match reply_rx.await {
                    Ok(Ok(true)) => info!(target: "mimicwx::listen", "自动监听已添加: {target}"),
                    Ok(Ok(false)) => warn!(target: "mimicwx::listen", "自动监听失败: {target}"),
                    Ok(Err(e)) => warn!(target: "mimicwx::listen", "自动监听错误: {target} - {e}"),
                    Err(_) => warn!(target: "mimicwx::listen", "actor 响应通道已关闭"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }

            info!(target: "mimicwx::listen", "自动监听配置完成");
        });
    }

    // ⑪ 控制台命令读取器 (stdin)
    {
        let console_exit = exit_code.clone();
        let console_shutdown = shutdown_tx.clone();
        let console_wechat = wechat.clone();
        let console_tx = tx.clone();
        let console_input_tx = input_tx.clone();
        let console_config_path = config_path.clone();
        tokio::spawn(async move {
            console::console_loop(console_exit, console_shutdown, console_wechat, console_db_ref, console_tx, console_input_tx, console_config_path).await;
        });
    }

    // ⑫ 启动 HTTP 服务 (带优雅退出)
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(target: "mimicwx::init", "控制台命令: /restart /stop /status /refresh /help");

    let mut shutdown_rx = shutdown_tx_clone.subscribe();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!(target: "mimicwx::init", "收到关闭信号, 停止服务");
                }
                _ = tokio::signal::ctrl_c() => {
                    info!(target: "mimicwx::init", "收到 Ctrl+C, 停止服务");
                }
            }
        })
        .await?;

    let code = exit_code.load(Ordering::Relaxed);
    if code == 42 {
        info!(target: "mimicwx::init", "MimicWX 准备重启...");
    } else {
        info!(target: "mimicwx::init", "MimicWX 已停止");
    }
    std::process::exit(code);
}

fn find_db_dir() -> Option<PathBuf> {
    let mut candidates: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

    let mut search_dirs = std::collections::HashSet::new();
    search_dirs.insert(PathBuf::from("/home/wechat/Documents/xwechat_files"));
    search_dirs.insert(dirs_or_home().join("Documents/xwechat_files"));
    if let Ok(homes) = std::fs::read_dir("/home") {
        for h in homes.flatten() {
            search_dirs.insert(h.path().join("Documents/xwechat_files"));
        }
    }

    for xwechat_dir in &search_dirs {
        if let Ok(entries) = std::fs::read_dir(xwechat_dir) {
            for entry in entries.flatten() {
                let db_storage = entry.path().join("db_storage");
                if db_storage.exists() {
                    let msg_dir = db_storage.join("message");
                    let mtime = msg_dir.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::UNIX_EPOCH);
                    debug!(target: "mimicwx::init", "候选目录: {} (mtime={:?})", db_storage.display(), mtime);
                    candidates.push((db_storage, mtime));
                }
            }
        }
    }

    if !candidates.is_empty() {
        candidates.sort_by(|a, b| b.1.cmp(&a.1));
        let chosen = &candidates[0].0;
        if candidates.len() > 1 {
            info!(target: "mimicwx::init", "发现 {} 个账号目录, 选择最新的: {}", candidates.len(), chosen.display());
        } else {
            info!(target: "mimicwx::init", "数据库目录: {}", chosen.display());
        }
        return Some(chosen.clone());
    }

    let old_path = PathBuf::from("/home/wechat/.local/share/weixin/data/db_storage");
    if old_path.exists() {
        info!(target: "mimicwx::init", "数据库目录 (旧格式): {}", old_path.display());
        return Some(old_path);
    }

    None
}

fn dirs_or_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/root"))
}
