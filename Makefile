.PHONY: help fmt fmt-check clippy test check build run up down logs ps migrate secret obs-up obs-down obs-logs

help: ## 显示可用命令
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

fmt: ## 格式化代码
	cargo fmt --all

fmt-check: ## 检查格式（CI 使用）
	cargo fmt --all -- --check

clippy: ## 静态检查
	cargo clippy --workspace --all-targets -- -D warnings

test: ## 运行单元测试
	cargo test --workspace --locked

check: fmt-check clippy test ## 本地完整检查

build: ## 构建 release 二进制
	cargo build --release --locked

run: ## 本地启动（需要 .env 中的 PG/Redis）
	cargo run

up: ## 构建并启动完整环境（nginx + 2 个 API 副本 + PG + Redis）
	docker compose up -d --build

down: ## 停止并移除容器（保留数据卷）
	docker compose down

logs: ## 跟踪 API 日志
	docker compose logs -f api

ps: ## 查看服务状态
	docker compose ps

migrate: ## 手动执行迁移（需要 sqlx-cli）
	sqlx migrate run

secret: ## 生成 JWT_SECRET
	@openssl rand -hex 32

obs-up: ## 启用可选的 Grafana/Loki/Alloy 日志栈
	docker compose -f compose.yaml -f compose.observability.yaml --profile observability up -d

obs-down: ## 只停止可观测性组件
	docker compose -f compose.yaml -f compose.observability.yaml --profile observability stop loki alloy grafana

obs-logs: ## 跟踪可观测性组件日志
	docker compose -f compose.yaml -f compose.observability.yaml --profile observability logs -f loki alloy grafana
