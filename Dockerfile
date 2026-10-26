# 多阶段构建 Dockerfile（完全修复版）
# 前端构建阶段
FROM node:22-alpine AS frontend-builder

# 安装 pnpm
RUN npm install -g pnpm@9

# 设置工作目录
WORKDIR /app

# 复制依赖文件
COPY package.json pnpm-lock.yaml ./

# 安装依赖（使用缓存挂载）
RUN --mount=type=cache,target=/root/.pnpm-store,id=pnpm_cache \
    pnpm install --frozen-lockfile

# 复制所有源文件
COPY . .

# 构建前端（显示详细输出）
RUN pnpm run build && \
    ls -la dist/ && \
    echo "Frontend build completed successfully"

# Rust 后端构建阶段
FROM rust:1.86-slim AS backend-builder

# rust:1.84-slim 镜像中 Cargo 已经在 /usr/local/cargo/bin
# 不需要设置额外的环境变量

WORKDIR /app

# 安装构建依赖
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libssl3 \
    perl \
    make \
    gcc \
    g++ \
    && rm -rf /var/lib/apt/lists/*

# 验证 Rust 工具链
RUN rustc --version && cargo --version

# 复制 Cargo 配置文件
COPY server/Cargo.toml server/Cargo.lock ./

# 创建占位符源文件用于预构建依赖
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    echo "#![allow(dead_code)]" > src/lib.rs

# 预构建依赖（使用 BuildKit 缓存）
RUN --mount=type=cache,target=/app/target,id=cargo_cache,sharing=locked \
    cargo build --release && \
    echo "Dependency build completed successfully"

# 删除占位符源文件
RUN rm -rf src

# 复制实际源代码
COPY server/src ./src

# 构建应用（使用缓存）- 只构建 release 版本
RUN --mount=type=cache,target=/app/target,id=cargo_cache,sharing=locked \
    cargo build --release && \
    ls -lh target/release/server && \
    echo "Backend build completed successfully"

# 将构建产物从缓存目录复制到镜像层（确保 COPY --from=backend-builder 可以访问）
RUN --mount=type=cache,target=/app/target,id=cargo_cache,sharing=locked \
    cp target/release/server /app/server && \
    ls -lh /app/server && \
    echo "Binary copied successfully"

# Native kbsync helper (source-only: Apple runtime assets are fetched on first use)
FROM golang:1.26.5-bookworm@sha256:53eeac89074db483fdf0ab3be1df32bf6e47562263d2d0d6baa7f26acb4957dd AS kbsync-builder
RUN apt-get update && apt-get install -y --no-install-recommends python3-minimal && rm -rf /var/lib/apt/lists/*
WORKDIR /helper
COPY scripts/kbsync-helper/ ./
RUN python3 verify-source.py && go mod verify && go test -mod=readonly ./... && go build -mod=readonly -trimpath -buildvcs=false -o /ipatool-kbsync .

# 生产环境镜像
FROM debian:bookworm-slim

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

# 设置工作目录
WORKDIR /app

# 从 frontend-builder 阶段复制构建好的前端文件
COPY --from=frontend-builder /app/dist ./dist

# 从 backend-builder 阶段复制构建好的后端二进制文件
COPY --from=backend-builder /app/server ./server
COPY --from=kbsync-builder /ipatool-kbsync ./ipatool-kbsync
ENV XDG_CACHE_HOME=/app/data/cache

# 创建数据目录并设置权限
RUN mkdir -p /app/data && \
    chmod 777 /app/data

# 暴露端口
EXPOSE 8080

# 设置环境变量
ENV PORT=8080
ENV RUST_LOG=info
ENV DATABASE_PATH=/app/data/ipa-webtool.db

# 健康检查
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS "http://localhost:${PORT:-8080}/health" || exit 1

# 运行应用
CMD ["./server"]
