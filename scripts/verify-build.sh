#!/bin/bash
# Docker构建验证脚本

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

echo "=== 开始验证Docker构建 ==="

# 1. 检查必要文件
echo "检查必要文件..."
required_files=(
    "Dockerfile"
    "package.json"
    "pnpm-lock.yaml"
    "server/Cargo.toml"
    "server/Cargo.lock"
    "server/src/main.rs"
    "frontend/App.vue"
)

for file in "${required_files[@]}"; do
    if [ -f "$file" ]; then
        echo "✓ $file"
    else
        echo "✗ $file 缺失"
        exit 1
    fi
done

# 2. 验证前端依赖
echo ""
echo "验证前端依赖..."
if pnpm install --frozen-lockfile --ignore-scripts; then
    echo "✓ 前端依赖检查通过"
else
    echo "✗ 前端依赖检查失败"
    exit 1
fi

# 3. 验证Rust依赖
echo ""
echo "验证Rust依赖..."
if cargo check --locked --manifest-path server/Cargo.toml; then
    echo "✓ Rust依赖检查通过"
else
    echo "✗ Rust依赖检查失败"
    exit 1
fi

# 4. 检查Dockerfile语法
echo ""
echo "检查Dockerfile语法..."
if docker build --check -f Dockerfile . 2>&1; then
    echo "✓ Dockerfile语法检查通过"
else
    echo "✗ Dockerfile检查失败（需要支持 --check 的 Docker BuildKit）"
    exit 1
fi

echo ""
echo "=== 验证完成 ==="
echo "所有检查通过，可以尝试构建Docker镜像"
echo ""
echo "构建命令："
echo "  docker build -t ipa-webtool:test ."
echo ""
echo "多平台构建命令："
echo "  docker buildx build --platform linux/amd64,linux/arm64 -t ipa-webtool:test ."
