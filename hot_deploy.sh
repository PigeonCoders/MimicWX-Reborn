#!/bin/bash
# MimicWX 热部署脚本
# 在容器内编译最新源码并重启 MimicWX 进程，无需重建镜像
#
# 用法: ./hot_deploy.sh
#
# 首次运行会自动安装 Rust 工具链 (~2分钟)
# 后续运行增量编译 (~10-30秒)

set -e

CONTAINER="mimicwx-linux"
CARGO_HOME="/opt/cargo-cache"
SRC_DIR="/home/wechat/mimicwx-reborn"
BIN_PATH="/usr/local/bin/mimicwx"

# 颜色
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()  { echo -e "${GREEN}[hot_deploy]${NC} $*"; }
warn() { echo -e "${YELLOW}[hot_deploy]${NC} $*"; }
err()  { echo -e "${RED}[hot_deploy]${NC} $*"; }

# 检查容器是否运行
if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER}$"; then
  err "容器 ${CONTAINER} 未运行"
  exit 1
fi

# ============================================================
# 1) 安装 Rust 工具链 (首次, 持久化到 cargo-cache volume)
# ============================================================
if ! docker exec "$CONTAINER" test -f "${CARGO_HOME}/bin/cargo"; then
  log "首次运行, 安装 Rust 工具链..."
  
  # 安装编译依赖
  docker exec "$CONTAINER" bash -c "
    apt-get update -qq && \
    apt-get install -y -qq --no-install-recommends \
      build-essential pkg-config curl ca-certificates \
      libdbus-1-dev libatspi2.0-dev libglib2.0-dev 2>&1 | tail -3
  "
  
  # 安装 Rust 到持久化 volume (TMPDIR 避免 /tmp noexec 问题)
  docker exec "$CONTAINER" bash -c "
    mkdir -p '${CARGO_HOME}/tmp'
    export TMPDIR='${CARGO_HOME}/tmp'
    export CARGO_HOME='${CARGO_HOME}'
    export RUSTUP_HOME='${CARGO_HOME}/rustup'
    export RUSTUP_DIST_SERVER=https://rsproxy.cn
    export RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup
    curl -sSf https://rsproxy.cn/rustup-init.sh | sh -s -- -y --default-toolchain stable --profile minimal 2>&1 | tail -5
  "
  
  # 配置 cargo 镜像
  docker exec "$CONTAINER" bash -c "
    mkdir -p ${CARGO_HOME}
    cat > ${CARGO_HOME}/config.toml << 'EOF'
[source.crates-io]
replace-with = \"ustc\"
[source.ustc]
registry = \"sparse+https://mirrors.ustc.edu.cn/crates.io-index/\"
EOF
  "
  
  log "Rust 工具链安装完成"
fi

# ============================================================
# 2) 容器内编译
# ============================================================
log "编译中..."
BUILD_START=$(date +%s)

docker exec "$CONTAINER" bash -c "
  export CARGO_HOME='${CARGO_HOME}'
  export RUSTUP_HOME='${CARGO_HOME}/rustup'
  export PATH='${CARGO_HOME}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin'
  cd '${SRC_DIR}'
  cargo build --release 2>&1
"
BUILD_STATUS=$?

BUILD_END=$(date +%s)
BUILD_TIME=$((BUILD_END - BUILD_START))

if [ $BUILD_STATUS -ne 0 ]; then
  err "编译失败 (${BUILD_TIME}s)"
  exit 1
fi
log "编译成功 (${BUILD_TIME}s)"

# ============================================================
# 3) 停止旧进程 + 替换二进制 + 重启
# ============================================================
log "停止旧进程..."
docker exec "$CONTAINER" bash -c "
  PID=\$(pgrep -f '${BIN_PATH}' | head -1)
  if [ -n \"\$PID\" ]; then
    kill \$PID 2>/dev/null
    echo \"已终止旧进程 (PID=\$PID)\"
    sleep 1
  else
    echo \"未找到运行中的 MimicWX 进程\"
  fi
"

log "替换二进制..."
docker exec "$CONTAINER" bash -c "
  cp '${SRC_DIR}/target/release/mimicwx' '${BIN_PATH}'
  chmod +x '${BIN_PATH}'
  setcap cap_sys_admin+ep '${BIN_PATH}'
"
# start.sh 重启循环会自动拉起新进程 (exit code 143 = SIGTERM)

# 等待新进程启动
log "等待新进程启动..."
for i in $(seq 1 15); do
  sleep 2
  status=$(curl -sf http://localhost:8899/status 2>/dev/null || true)
  if [ -n "$status" ]; then
    version=$(echo "$status" | python3 -c "import json,sys; print(json.load(sys.stdin).get('version','?'))" 2>/dev/null)
    log "✅ 热部署完成! v${version} (编译${BUILD_TIME}s + 启动$((i*2))s)"
    exit 0
  fi
done

warn "MimicWX 进程未在 30s 内响应, 可能需要扫码登录微信"
warn "检查: docker logs ${CONTAINER} --tail 10"
