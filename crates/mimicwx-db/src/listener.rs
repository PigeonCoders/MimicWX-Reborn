//! WAL 文件监听 (fanotify + PID 过滤)
//!
//! 使用 fanotify FAN_MARK_MOUNT 监听 message 目录下所有文件的修改事件,
//! 通过 PID 过滤丢弃自身进程触发的事件, 消除自循环。
//! 独立线程运行, 通过 broadcast 通道通知多消费者。

use anyhow::{Context, Result};
use std::path::Path;
use tokio::sync::broadcast;
use tracing::{info, trace};

/// WAL 目录/文件等待轮询间隔
const WAL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// 启动 WAL 文件监听循环 (在独立线程中运行)
///
/// 返回 `Result<()>` —— 出错时退出并记录日志。
/// 通过 `tx` 广播通知所有消费者。
pub fn wal_watch_loop(db_dir: &Path, tx: broadcast::Sender<()>) -> Result<()> {
    use fanotify::high_level::*;

    let self_pid = std::process::id() as i32;
    info!(target: "mimicwx::db", "fanotify PID 过滤: self_pid={self_pid}");

    let msg_dir = db_dir.join("message");

    if !msg_dir.exists() {
        info!(target: "mimicwx::db", "等待 message 目录: {}", msg_dir.display());
        loop {
            std::thread::sleep(WAL_POLL_INTERVAL);
            if msg_dir.exists() {
                info!(target: "mimicwx::db", "message 目录已创建");
                break;
            }
        }
    }

    let wal_path = msg_dir.join("message_0.db-wal");
    if !wal_path.exists() {
        info!(target: "mimicwx::db", "等待 WAL 文件: {}", wal_path.display());
        loop {
            std::thread::sleep(WAL_POLL_INTERVAL);
            if wal_path.exists() {
                info!(target: "mimicwx::db", "WAL 文件已创建");
                break;
            }
        }
    }

    let fan = Fanotify::new_blocking(FanotifyMode::NOTIF)
        .with_context(|| "fanotify 初始化失败")?;

    fan.add_mountpoint(FanEvent::Modify.into(), &msg_dir)
        .with_context(|| format!("fanotify add_mountpoint 失败: {}", msg_dir.display()))?;

    info!(target: "mimicwx::db", "开始监听 WAL: {} (fanotify)", wal_path.display());

    let msg_dir_prefix = msg_dir.to_string_lossy().to_string();

    loop {
        let events = fan.read_event();

        let mut has_external_modify = false;
        for event in events {
            if event.pid == self_pid {
                continue;
            }

            if !event.path.starts_with(&msg_dir_prefix) {
                continue;
            }

            trace!(target: "mimicwx::db", "外部 MODIFY pid={}: {}", event.pid, event.path);
            has_external_modify = true;
        }

        if has_external_modify {
            let _ = tx.send(());
        }
    }
}
