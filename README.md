# MimicWX

**零风险微信自动化框架 v0.6.0** — 基于 AT-SPI2 无障碍接口 + X11 XTEST 输入注入 + SQLCipher 数据库解密

> Zero-risk WeChat automation framework for Linux. 6-crate workspace, 34 modules, 55 tests.

---

## 架构

```
mimicwx-app (binary)
├── mimicwx-wechat    # 微信业务逻辑 (会话/发送/监听)
│   ├── mimicwx-atspi # AT-SPI2 底层原语 (D-Bus 通信)
│   ├── mimicwx-input # X11 XTEST 输入注入 (键鼠/剪贴板/窗口)
│   └── mimicwx-core  # 基础层 (error/config/types/timing)
└── mimicwx-db        # SQLCipher 数据库监听 (WAL/联系人/消息解析)
    └── mimicwx-core
```

**依赖层级**: core (L0) → atspi + input (L1) → db (L2) → wechat (L3) → app (L4)

---

## 特性

- **数据库消息检测** — SQLCipher 解密 WCDB + fanotify WAL 实时监听，亚秒级延迟，16+ 种消息类型结构化解析
- **X11 原生输入注入** — XTEST 扩展注入键鼠事件 + X11 Selection 协议直接操作剪贴板（零外部进程依赖）
- **自动密钥提取** — 进程内存扫描 + HMAC 验证，扫码登录后自动提取 32 字节 AES 密钥，支持密钥过期自动更新
- **独立聊天窗口** — 借鉴 [wxauto](https://github.com/cluic/wxauto) 的 ChatWnd 设计，多窗口并行收发 + 缓存节点自动失效重建
- **REST + WebSocket API** — 完整 HTTP API + WebSocket 实时推送 (30s 心跳)，CORS 全开放，可对接 Yunzai 等机器人框架
- **Docker 一键部署** — 多阶段构建 + Xvfb/VNC 虚拟桌面，开箱即用
- **Token 认证** — Bearer Token 认证保护 API 安全
- **交互式控制台** — 支持 `/restart`、`/stop`、`/status`、`/refresh`、`/help` 等命令，方向键切换历史
- **自动弹性** — AT-SPI2 心跳自动重连、密钥过期自愈、独立窗口弹出重试、联系人定时刷新、优雅重启/关闭

---

## 项目结构

```
MimicWX-Reborn/
├── crates/
│   ├── mimicwx-core/         # 基础层 (error/config/types/timing/predicates)
│   │   └── src/              # 5 模块, 19 测试
│   ├── mimicwx-atspi/        # AT-SPI2 底层原语
│   │   └── src/              # 6 模块 (connection/node/search/dump/helpers/traits)
│   ├── mimicwx-input/        # X11 XTEST 输入引擎
│   │   └── src/              # 5 模块 (keyboard/clipboard/mouse/window/traits)
│   ├── mimicwx-db/           # SQLCipher 数据库监听
│   │   └── src/              # 7 模块 (types/key/wcdb/parser/listener/contacts/manager)
│   ├── mimicwx-wechat/       # 微信业务逻辑
│   │   └── src/              # 8 模块 (types/chatwnd/manager/status/control/session/listen/send)
│   └── mimicwx-app/          # 应用入口
│       └── src/              # 3 模块 (api/console/main)
├── docker/
│   ├── start.sh              # 容器启动脚本
│   ├── extract_key.py        # GDB 密钥提取脚本
│   └── dbus-mimicwx.conf     # D-Bus 配置 (允许 eavesdrop)
├── adapter/
│   └── MimicWX.js            # Yunzai-Bot 适配器
├── Cargo.toml                # Workspace 根配置
├── Dockerfile                # 多阶段构建 (workspace 适配)
├── docker-compose.yml        # 编排配置
├── config.toml               # 运行时配置文件
└── hot_deploy.sh             # 热部署脚本
```

---

## 核心模块详解

### `mimicwx-core` — 基础层

| 模块 | 说明 |
|------|------|
| `error` | `MimicError` 枚举 (thiserror) + `Result<T>` 别名 |
| `config` | 配置文件管理 (3路径搜索, 保存保留注释) |
| `types` | 共享类型: `BBox`(矩形)/`SearchAction`(搜索动作)/`TreeNode`(树节点) |
| `timing` | `ms(u64) -> Duration` |
| `predicates` | 谓词函数: `is_message_list`/`is_structural_role` |

### `mimicwx-atspi` — AT-SPI2 底层原语

| 模块 | 说明 |
|------|------|
| `connection` | `AtSpi` 结构体, 4级连接策略, 运行时重连, 500ms 超时 |
| `node` | `NodeRef` (bus + path) + `registry()` |
| `search` | 泛型 BFS/DFS 搜索 (兼容 `&dyn AtspiClient`) |
| `dump` | `dump_tree` 控件树导出 (max 200 节点) |
| `helpers` | `verify_sent_in_list`/`wait_for`/`wait_for_result` |
| `traits` | `AtspiClient` trait (async_trait, 12方法, 对象安全) |

### `mimicwx-input` — X11 XTEST 输入引擎

| 模块 | 说明 |
|------|------|
| `keyboard` | `InputEngine` + 按键映射, press_key/key_combo/type_text |
| `clipboard` | X11 Selection 协议粘贴 (中文/图片, 零外部进程) |
| `mouse` | 移动/单击/双击/右键/滚轮 |
| `window` | 原生 `_NET_ACTIVE_WINDOW` 激活/`_NET_CLOSE_WINDOW` 关闭 |
| `traits` | `InputDevice` trait (async_trait, 14方法, Send+Sync+'static) |

### `mimicwx-db` — 数据库监听

| 模块 | 说明 |
|------|------|
| `types` | `ContactInfo`/`MsgContent`(11变体)/`DbMessage` |
| `key` | SQLCipher 密钥管理 (FFI/三级匹配: 专属→默认→暴力) |
| `wcdb` | WCDB 兼容 (Zstd解压/TEXT+BLOB/表发现/MD5缓存) |
| `parser` | 16+ 种消息类型解析 (quick-xml + protobuf) |
| `listener` | fanotify WAL 监听 (PID过滤+broadcast通知) |
| `contacts` | 联系人/群成员 + `refresh_contacts` (ArcSwap无锁快照) |
| `manager` | `DbManager` 主结构 (连接池/增量消息/发送验证/全部已读) |

### `mimicwx-wechat` — 微信业务逻辑

| 模块 | 说明 |
|------|------|
| `types` | `WeChatStatus`/`SessionInfo`/`CachedNode`(TTL缓存) |
| `chatwnd` | `ChatWnd` 独立聊天窗口 (缓存失效自动重搜) |
| `manager` | `WeChat` 主结构体 (6字段, pub(crate)跨模块impl) |
| `status` | 状态检测/应用查找 (30s缓存)/AT-SPI2重连 |
| `control` | 控件查找 (split pane/会话列表/消息列表/输入框) |
| `session` | 会话切换 ChatWith (快速路径→列表→Ctrl+F回退) |
| `listen` | 独立窗口管理 (双击弹出+3次重试/关闭/存活检测/自动恢复) |
| `send` | 消息发送 (优先独立窗口→@提及→粘贴→Enter→验证) |

### `mimicwx-app` — 应用入口

| 模块 | 说明 |
|------|------|
| `api` | HTTP+WebSocket API (axum, 15端点, Actor模式, Bearer认证) |
| `console` | 交互式控制台 (raw mode, 行编辑, 10个命令, UTF-8中文) |
| `main` | 启动编排(12步), CompactFormat日志, 优雅退出 |

---

## 快速开始

### 环境要求

- Linux 系统 (Ubuntu 22.04+ 推荐)
- Docker + Docker Compose
- 允许 `SYS_ADMIN` / `SYS_PTRACE` 能力

### 一键部署

```bash
git clone https://github.com/PigeonCoders/MimicWX-Reborn.git
cd MimicWX-Reborn
docker compose up -d
```

### 首次使用

1. 打开 noVNC: `http://HOST:6080/vnc.html` (密码: `mimicwx`)
2. 在虚拟桌面中扫码登录微信
3. GDB 自动提取数据库密钥 → MimicWX 自动启动
4. 通过 API 接口开始使用

### 访问入口

| 服务 | 地址 | 说明 |
|------|------|------|
| noVNC | `http://HOST:6080/vnc.html` | 浏览器远程桌面 (密码: `mimicwx`) |
| VNC | `vnc://HOST:5901` | VNC 客户端连接 |
| API | `http://HOST:8899` | REST API 接口 |
| WebSocket | `ws://HOST:8899/ws` | 实时消息推送 |

---

## 配置文件

`config.toml` — 配置搜索优先级: `./config.toml` → `/home/wechat/mimicwx-reborn/config.toml` → `/etc/mimicwx/config.toml`

```toml
[api]
token = "your-secret-token"

[listen]
auto = ["文件传输助手", "好友A", "工作群"]

[timing]
at_delay_ms = 300
```

---

## API 端点

| 端点 | 方法 | 说明 | 认证 |
|------|------|------|------|
| `/status` | GET | 服务状态 + DB/联系人/运行时间 | 否 |
| `/contacts` | GET | 联系人列表 (数据库) | 是 |
| `/group_members` | GET | 群成员列表 | 是 |
| `/sessions` | GET | 会话列表 (优先数据库) | 是 |
| `/messages/new` | GET | 新消息 (数据库增量) | 是 |
| `/send` | POST | 发送文本消息 (限10KB) | 是 |
| `/send_image` | POST | 发送图片 (base64, 限27MB) | 是 |
| `/chat` | POST | 切换聊天目标 | 是 |
| `/listen` | GET/POST/DELETE | 监听管理 | 是 |
| `/command` | POST | 通用命令执行 | 是 |
| `/ws` | GET | WebSocket 实时消息推送 | 是 |
| `/debug/tree` | GET | AT-SPI2 控件树 | 是 |
| `/debug/sessions` | GET | 会话容器子树 | 是 |

---

## 对接 Yunzai-Bot

```bash
export MIMICWX_URL="http://localhost:8899"
export MIMICWX_TOKEN="your-secret-token"
```

---

## 控制台命令

通过 `docker attach` 进入交互式控制台：

| 命令 | 功能 |
|------|------|
| `/restart` | 优雅重启 (返回重启循环) |
| `/stop` | 正常关闭 |
| `/status` | 显示运行时状态 |
| `/refresh` | 手动刷新联系人 |
| `/reload` | 热重载配置 (diff 监听列表) |
| `/atmode` | 切换仅@模式 |
| `/send <收件人> <内容>` | 发送消息 |
| `/listen <名称>` | 添加监听 (自动持久化) |
| `/unlisten <名称>` | 移除监听 (自动持久化) |
| `/sessions` | 查看会话列表 |
| `/help` | 显示帮助 |

---

## 技术栈

| 组件 | 技术 |
|------|------|
| 语言 | Rust 2021 edition |
| 异步运行时 | Tokio (全功能) |
| 错误处理 | thiserror (core) + anyhow (app) |
| 消息检测 | SQLCipher + fanotify |
| UI 自动化 | AT-SPI2 (atspi-rs + zbus) |
| 输入注入 | X11 XTEST (x11rb) |
| API 服务 | axum 0.8 + tower-http (CORS) |
| 序列化 | serde + serde_json |
| XML 解析 | quick-xml |
| 压缩 | zstd (WCDB BLOB) |
| 容器化 | Docker (Ubuntu 22.04) |
| 虚拟桌面 | TigerVNC + noVNC |
| 密钥提取 | GDB + Python |

---

## License

MIT
