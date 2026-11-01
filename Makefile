.PHONY: help init fmt fmt-check clippy test check build run dev-up dev-down mail-up integration up down logs ps migrate secret obs-up obs-down obs-logs web-dev web-build web-check web-docker room-agent-build agent-build

init: ## 从模板初始化（make init SLUG=acme NAME="Acme"）
	@test -n "$(SLUG)" || (echo "用法: make init SLUG=acme NAME='Acme' [REPO=https://github.com/acme/backend]"; exit 1)
	scripts/init-template.sh "$(SLUG)" "$(NAME)" "$(REPO)"

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
	cargo build --release --locked -p qe-api

run: ## 本地启动（需要 .env 中的 PG/Redis）
	cargo run -p qe-api

up: ## 构建并启动完整环境（nginx + 2 个 API 副本 + PG + Redis）
	docker compose up -d --build

dev-up: ## 只启动本地开发依赖 PG/Redis
	docker compose up -d postgres redis
	@echo "依赖已启动。下一步：cargo run -p qe-api"

dev-down: ## 停止本地开发依赖（保留数据卷）
	docker compose stop postgres redis

mail-up: ## 启动本地 Mailpit（SMTP 1025 / Web 8025）
	docker compose -f compose.yaml -f compose.mail.yaml up -d mailpit

integration: ## 真实 PostgreSQL/Redis 集成测试
	scripts/test-integration.sh

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

web-dev: ## 启动管理控制台开发服务器（需先 make run；默认 127.0.0.1:5174）
	cd frontend && ./node_modules/.bin/vite --host 127.0.0.1 --port 5174

web-build: ## 构建管理控制台静态资源到 frontend/dist
	cd frontend && ./node_modules/.bin/vite build

web-check: ## 管理控制台类型检查 + 单元测试
	cd frontend && ./node_modules/.bin/vue-tsc --noEmit -p tsconfig.app.json
	cd frontend && ./node_modules/.bin/vitest run

room-agent-build: ## 构建 room-agent（房间机器遥测 agent）
	cargo build -p qe-room-agent

agent-build: ## 构建统一 agent 客户端（qe-agent：enroll / node / machine）
	cargo build -p qe-agent

web-docker: ## 构建并启动控制台容器（http://localhost:8081，需先 make up）
	docker compose --profile console up -d --build console
