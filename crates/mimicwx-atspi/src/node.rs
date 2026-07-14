//! AT-SPI2 节点引用

use zbus::zvariant::OwnedObjectPath;

/// AT-SPI2 节点引用 (bus_name + object_path)
#[derive(Debug, Clone)]
pub struct NodeRef {
    pub bus: String,
    pub path: OwnedObjectPath,
}

/// AT-SPI2 Registry 根节点
pub fn registry() -> Option<NodeRef> {
    Some(NodeRef {
        bus: "org.a11y.atspi.Registry".into(),
        path: "/org/a11y/atspi/accessible/root".try_into().ok()?,
    })
}
