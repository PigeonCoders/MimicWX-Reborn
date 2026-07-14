//! AT-SPI2 连接管理
//!
//! 封装 zbus D-Bus 连接, 提供多种连接策略:
//! 1. 通过 session bus 上的 org.a11y.Bus 接口获取 AT-SPI2 bus 地址
//! 2. 使用 AT_SPI_BUS_ADDRESS 环境变量
//! 3. 标准 AccessibilityConnection (自动发现)
//! 4. 扫描 ~/.cache/at-spi/ 下所有 bus socket
//!
//! 支持运行时重连: 当检测到 Registry 为空时可调用 [`AtSpi::reconnect`] 重新发现。

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::{debug, info};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::node::{NodeRef, registry};

// D-Bus 常量
pub(crate) const IFACE_ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
pub(crate) const IFACE_COMPONENT: &str = "org.a11y.atspi.Component";
pub(crate) const IFACE_TEXT: &str = "org.a11y.atspi.Text";
pub(crate) const PROPS: &str = "org.freedesktop.DBus.Properties";
pub(crate) const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

/// AT-SPI2 客户端
///
/// 所有 D-Bus 调用带 500ms 超时保护 ([`CALL_TIMEOUT`])。
/// 连接通过 [`AtSpi::connect`] 建立, 支持运行时 [`AtSpi::reconnect`]。
pub struct AtSpi {
    pub(crate) conn: RwLock<zbus::Connection>,
}

impl AtSpi {
    /// 建立 AT-SPI2 连接
    ///
    /// 尝试多种发现方式, 返回第一个有应用注册的连接。
    /// 所有方式都失败时回退到标准发现 (等待应用注册)。
    pub async fn connect() -> Result<Self> {
        if let Some(instance) = Self::try_connect_all().await {
            return Ok(instance);
        }

        let a11y = atspi::AccessibilityConnection::new().await?;
        let conn = a11y.connection().clone();
        info!(target: "mimicwx::atspi", "连接就绪 (标准发现, 等待应用注册)");
        Ok(Self { conn: RwLock::new(conn) })
    }

    /// 尝试所有连接方式，返回第一个有应用注册的连接
    async fn try_connect_all() -> Option<Self> {
        if let Some(instance) = Self::connect_via_a11y_bus().await {
            return Some(instance);
        }

        if let Ok(addr) = std::env::var("AT_SPI_BUS_ADDRESS") {
            if !addr.is_empty() {
                debug!(target: "mimicwx::atspi", "尝试 AT_SPI_BUS_ADDRESS: {addr}");
                if let Some(instance) = Self::connect_to_address(&addr).await {
                    info!(target: "mimicwx::atspi", "连接就绪 (AT_SPI_BUS_ADDRESS)");
                    return Some(instance);
                }
            }
        }

        if let Ok(a11y) = atspi::AccessibilityConnection::new().await {
            let conn = a11y.connection().clone();
            let instance = Self { conn: RwLock::new(conn) };
            if let Some(root) = registry() {
                let count = instance.child_count(&root).await;
                if count > 1 {
                    info!(target: "mimicwx::atspi", "连接就绪 (标准, {count} 个应用)");
                    return Some(instance);
                }
                debug!(target: "mimicwx::atspi", "标准连接只有 {count} 个子节点");
            }
        }

        if let Some(instance) = Self::scan_bus_sockets().await {
            info!(target: "mimicwx::atspi", "连接就绪 (扫描发现)");
            return Some(instance);
        }

        None
    }

    /// 通过 session bus 上 org.a11y.Bus 接口获取 AT-SPI2 bus 地址
    async fn connect_via_a11y_bus() -> Option<Self> {
        debug!(target: "mimicwx::atspi", "通过 org.a11y.Bus 发现 bus...");

        let session = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                debug!(target: "mimicwx::atspi", "session bus 连接失败: {e}");
                return None;
            }
        };

        let reply = match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            session.call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.a11y.Bus"),
                "GetAddress",
                &(),
            ),
        ).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                debug!(target: "mimicwx::atspi", "GetAddress 调用失败: {e}");
                return None;
            }
            Err(_) => {
                debug!(target: "mimicwx::atspi", "GetAddress 超时");
                return None;
            }
        };

        let addr: String = reply.body().deserialize().ok()?;
        if addr.is_empty() {
            debug!(target: "mimicwx::atspi", "Bus 返回空地址");
            return None;
        }

        info!(target: "mimicwx::atspi", "AT-SPI2 bus: {addr}");
        Self::connect_to_address(&addr).await
    }

    /// 连接到指定地址的 AT-SPI2 bus
    async fn connect_to_address(addr: &str) -> Option<Self> {
        let socket_path = if let Some(rest) = addr.strip_prefix("unix:path=") {
            rest.split(',').next()?.to_string()
        } else {
            debug!(target: "mimicwx::atspi", "不支持的地址格式: {addr}");
            return None;
        };

        debug!(target: "mimicwx::atspi", "连接 socket: {socket_path}");

        let stream = match tokio::net::UnixStream::connect(&socket_path).await {
            Ok(s) => s,
            Err(e) => {
                debug!(target: "mimicwx::atspi", "socket 连接失败: {e}");
                return None;
            }
        };

        let conn = match zbus::connection::Builder::unix_stream(stream)
            .build()
            .await
        {
            Ok(c) => c,
            Err(e) => {
                debug!(target: "mimicwx::atspi", "zbus 连接失败: {e}");
                return None;
            }
        };

        let instance = Self { conn: RwLock::new(conn) };
        if let Some(root) = registry() {
            let count = instance.child_count(&root).await;
            debug!(target: "mimicwx::atspi", "bus {socket_path} 有 {count} 个子节点");
            if count > 0 {
                info!(target: "mimicwx::atspi", "找到有效 bus: {socket_path} ({count} 个应用)");
                return Some(instance);
            }
        }

        debug!(target: "mimicwx::atspi", "bus {socket_path} 暂无应用, 保留连接");
        Some(instance)
    }

    /// 运行时重连: 重新发现 AT-SPI2 bus 并更新连接
    ///
    /// 当 Registry 持续返回 0 个子节点时调用此方法。
    pub async fn reconnect(&self) -> bool {
        info!(target: "mimicwx::atspi", "尝试重新发现 bus...");

        if let Some(new_conn) = Self::connect_via_a11y_bus().await {
            let new_inner = new_conn.conn.read().await.clone();
            if let Some(root) = registry() {
                let tmp = Self { conn: RwLock::new(new_inner.clone()) };
                let count = tmp.child_count(&root).await;
                if count > 0 {
                    let mut conn = self.conn.write().await;
                    *conn = new_inner;
                    info!(target: "mimicwx::atspi", "重连成功 (org.a11y.Bus, {count} 个应用)");
                    return true;
                }
            }
        }

        if let Some(new_conn) = Self::scan_bus_sockets().await {
            let new_inner = new_conn.conn.read().await.clone();
            let mut conn = self.conn.write().await;
            *conn = new_inner;
            info!(target: "mimicwx::atspi", "重连成功 (socket 扫描)");
            return true;
        }

        debug!(target: "mimicwx::atspi", "重连未发现新的 bus");
        false
    }

    /// 扫描 ~/.cache/at-spi/ 下的所有 bus socket 文件
    async fn scan_bus_sockets() -> Option<Self> {
        use std::os::unix::fs::FileTypeExt;

        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/wechat".into());
        let bus_dir = std::path::PathBuf::from(&home).join(".cache/at-spi");

        let entries = std::fs::read_dir(&bus_dir).ok()?;

        for entry in entries.flatten() {
            let path = entry.path();

            if let Ok(meta) = std::fs::metadata(&path) {
                if !meta.file_type().is_socket() {
                    continue;
                }
            } else {
                continue;
            }

            let path_str = path.to_string_lossy().to_string();
            debug!(target: "mimicwx::atspi", "尝试 bus: {path_str}");

            let stream = match tokio::net::UnixStream::connect(&path).await {
                Ok(s) => s,
                Err(e) => {
                    debug!(target: "mimicwx::atspi", "连接失败: {e}");
                    continue;
                }
            };

            let conn = match zbus::connection::Builder::unix_stream(stream)
                .build()
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    debug!(target: "mimicwx::atspi", "zbus 连接失败: {e}");
                    continue;
                }
            };

            let instance = Self { conn: RwLock::new(conn) };
            if let Some(root) = registry() {
                let count = instance.child_count(&root).await;
                if count > 1 {
                    info!(target: "mimicwx::atspi", "找到有效 bus: {path_str} ({count} 个应用)");
                    return Some(instance);
                }
                debug!(target: "mimicwx::atspi", "bus {path_str} 只有 {count} 个子节点, 跳过");
            }
        }
        None
    }

    // =================================================================
    // D-Bus 底层调用 (带超时)
    // =================================================================

    /// D-Bus 方法调用, 带 500ms 超时保护
    ///
    /// 超时或错误时返回 `None` 并记录 debug 日志。
    pub(crate) async fn call(
        &self, bus: &str, path: &str,
        iface: Option<&str>, method: &str,
        body: &(impl serde::Serialize + zbus::zvariant::DynamicType + Sync),
    ) -> Option<zbus::Message> {
        let conn = self.conn.read().await;
        match tokio::time::timeout(
            CALL_TIMEOUT,
            conn.call_method(Some(bus), path, iface, method, body),
        ).await {
            Ok(Ok(reply)) => Some(reply),
            Ok(Err(e)) => { debug!(target: "mimicwx::atspi", "D-Bus {method}: {e}"); None }
            Err(_) => { debug!(target: "mimicwx::atspi", "D-Bus {method}: timeout"); None }
        }
    }

    // =================================================================
    // 属性读取方法 (impl AtspiClient trait)
    // =================================================================

    pub async fn child_count(&self, node: &NodeRef) -> i32 {
        let reply = self.call(
            &node.bus, node.path.as_str(), Some(PROPS), "Get",
            &(IFACE_ACCESSIBLE, "ChildCount"),
        ).await;
        reply.and_then(|r| {
            let v: OwnedValue = r.body().deserialize().ok()?;
            i32::try_from(&v).ok()
                .or_else(|| u32::try_from(&v).ok().map(|n| n as i32))
        }).unwrap_or(0)
    }

    pub async fn child_at(&self, node: &NodeRef, idx: i32) -> Option<NodeRef> {
        let reply = self.call(
            &node.bus, node.path.as_str(),
            Some(IFACE_ACCESSIBLE), "GetChildAtIndex", &(idx,),
        ).await?;
        let (bus, path): (String, OwnedObjectPath) = reply.body().deserialize().ok()?;
        Some(NodeRef { bus, path })
    }

    pub async fn name(&self, node: &NodeRef) -> String {
        let reply = self.call(
            &node.bus, node.path.as_str(), Some(PROPS), "Get",
            &(IFACE_ACCESSIBLE, "Name"),
        ).await;
        reply.and_then(|r| {
            let v: OwnedValue = r.body().deserialize().ok()?;
            String::try_from(v).ok()
        }).unwrap_or_default()
    }

    pub async fn role(&self, node: &NodeRef) -> String {
        let reply = self.call(
            &node.bus, node.path.as_str(),
            Some(IFACE_ACCESSIBLE), "GetRoleName", &(),
        ).await;
        reply.and_then(|r| r.body().deserialize::<String>().ok())
            .unwrap_or_default()
    }

    /// 并发获取 role + name (2 次 D-Bus 调用并行)
    pub async fn role_and_name(&self, node: &NodeRef) -> (String, String) {
        tokio::join!(self.role(node), self.name(node))
    }

    pub async fn bbox(&self, node: &NodeRef) -> Option<BBox> {
        let reply = self.call(
            &node.bus, node.path.as_str(),
            Some(IFACE_COMPONENT), "GetExtents", &(0u32,),
        ).await?;
        let (x, y, w, h): (i32, i32, i32, i32) = reply.body().deserialize().ok()?;
        Some(BBox { x, y, w, h })
    }

    pub async fn text(&self, node: &NodeRef) -> Option<String> {
        let reply = self.call(
            &node.bus, node.path.as_str(),
            Some(IFACE_TEXT), "GetText", &(0i32, -1i32),
        ).await?;
        reply.body().deserialize::<String>().ok()
    }

    pub async fn description(&self, node: &NodeRef) -> String {
        let reply = self.call(
            &node.bus, node.path.as_str(), Some(PROPS), "Get",
            &(IFACE_ACCESSIBLE, "Description"),
        ).await;
        reply.and_then(|r| {
            let v: OwnedValue = r.body().deserialize().ok()?;
            String::try_from(v).ok()
        }).unwrap_or_default()
    }

    pub async fn parent(&self, node: &NodeRef) -> Option<NodeRef> {
        let reply = self.call(
            &node.bus, node.path.as_str(), Some(PROPS), "Get",
            &(IFACE_ACCESSIBLE, "Parent"),
        ).await?;
        let v: OwnedValue = reply.body().deserialize().ok()?;
        let (bus, path): (String, OwnedObjectPath) = zbus::zvariant::Value::try_from(v)
            .ok()
            .and_then(|v| v.downcast().ok())?;
        Some(NodeRef { bus, path })
    }

    /// 获取节点状态位集合 (AT-SPI2 StateSet, 两个 u32 合并为 u64)
    pub async fn get_states(&self, node: &NodeRef) -> u64 {
        let reply = self.call(
            &node.bus, node.path.as_str(),
            Some(IFACE_ACCESSIBLE), "GetState", &(),
        ).await;
        reply.and_then(|r| {
            let states: Vec<u32> = r.body().deserialize().ok()?;
            if states.len() >= 2 {
                Some((states[1] as u64) << 32 | states[0] as u64)
            } else if states.len() == 1 {
                Some(states[0] as u64)
            } else {
                None
            }
        }).unwrap_or(0)
    }

    /// 检查节点是否处于 SELECTED 状态 (AT-SPI2 STATE_SELECTED = bit 25)
    pub async fn is_selected(&self, node: &NodeRef) -> bool {
        let states = self.get_states(node).await;
        states & (1 << 25) != 0
    }

    /// 强制聚焦节点 (将窗口提到前台)
    pub async fn grab_focus(&self, node: &NodeRef) -> bool {
        let reply = self.call(
            &node.bus, node.path.as_str(),
            Some(IFACE_COMPONENT), "GrabFocus", &(),
        ).await;
        reply.and_then(|r| r.body().deserialize::<bool>().ok()).unwrap_or(false)
    }
}

// BBox 已在 mimicwx-core 中定义, 此处通过 use 引入供 call 方法使用
use mimicwx_core::BBox;
