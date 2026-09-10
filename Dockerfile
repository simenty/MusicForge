# syntax=docker/dockerfile:1
# MusicForge 容器交付（P8/X25-T3）：fnOS 原生 fpk 之外的通用 Linux 形态。
#
# 设计（P8 验收「镜像 <40MB 零插件」）：
# - 三阶段：node 构建前端 → alpine rust 构建 musl 静态二进制 → scratch 组装；
# - **零插件**：镜像不含任何插件运行时/插件二进制（插件独立分发，挂载白名单目录）；
# - **零网络**：镜像内无 curl/wget/shell（scratch）——业务网络面为零（R22/威胁模型）；
# - 容器惯例：监听 0.0.0.0（端口映射由 -p 控制），token 闸在位（首启随机生成于
#   /data/.token，`docker logs` 首启段可见）；R22 的回环默认仅适用于裸进程形态。

# ---- 阶段 1：前端 SPA ----
FROM node:22-alpine AS ui
WORKDIR /ui
COPY musicforge-gui/ui/package.json musicforge-gui/ui/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY musicforge-gui/ui/ ./
RUN npm run build

# ---- 阶段 2：musl 静态二进制（CLI + server）----
FROM rust:1-alpine AS build
# rusqlite bundled（自带 sqlite3.c，无需系统 sqlite）；无需 pkg-config
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
ENV CARGO_NET_OFFLINE=false
RUN cargo build --release -p musicforge-cli -p musicforge-server
# /data 数据目录预建 + 属主预置（scratch 无法 chown——匿名卷/挂载卷以镜像内
# 该路径权限初始化；缺失则 root:root → 非 root 运行写不进 → server 启动即退）
RUN mkdir -p /data && chown 65532:65532 /data

# ---- 阶段 3：scratch 组装 ----
FROM scratch
COPY --from=build /src/target/release/musicforge-server /usr/local/bin/musicforge-server
COPY --from=build /src/target/release/musicforge /usr/local/bin/musicforge
COPY --from=ui /ui/dist /opt/musicforge/ui
COPY --from=build --chown=65532:65532 /data /data
# 数据目录（token/日志/manifest）：挂载卷持久化
VOLUME ["/data"]
ENV MUSICFORGE_DATA_DIR=/data \
    MUSICFORGE_UI_DIR=/opt/musicforge/ui \
    MUSICFORGE_BIND=0.0.0.0:8787
# 非根运行（数字 UID——scratch 无 /etc/passwd）
USER 65532:65532
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/musicforge-server"]
