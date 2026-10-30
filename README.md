# QEStudio Backend

基于 **Rust + Axum + PostgreSQL + Redis** 的 Web API 服务，内置：

- 用户名/密码登录，JWT 访问令牌 + Redis 一次性刷新令牌
- 登录失败限制：Redis 计数 + 临时锁定，默认 5 次/5 分钟
- 密码重置、邮箱验证、多设备会话列表/撤销
- RBAC 权限模型（用户 → 角色 → 权限），管理员可在运行时改角色/权限
- 公告管理（草稿 / 发布 / 归档 / 置顶 / 分页 / 搜索），写入时清洗富文本
- 管理操作审计日志（用户/角色/公告），`audit:read` 权限查看
- 公告列表 Redis 代际缓存，变更时自动失效
- OpenAPI 文档：`/docs`，JSON：`/api-docs/openapi.json`
- 可选 OTLP/HTTP trace 导出（`OTEL_EXPORTER_OTLP_ENDPOINT`）
- Docker 多阶段构建，nginx 反向代理 + 多副本负载均衡
- GitHub Actions / Gitea Actions：格式检查、Clippy、单测、集成测试、镜像构建

## 从模板开始

1. 在 GitHub 点击 **Use this template**（或 clone 后删除 `.git`）。
2. 初始化项目标识：
   ```bash
   scripts/init-template.sh acme "Acme" https://github.com/acme/backend
   ```
   脚本会替换 `qestudio` / `QEStudio` / `qe-*`，并校验 Cargo manifest；`--yes` 跳过确认。
   `display-name` 用品牌名（如 `Acme`），不要带 `Backend`，否则 README 标题会重复。
3. 生成开发配置：
   ```bash
   cp .env.example .env
   make secret          # 生成 JWT_SECRET，手动写入 .env
   ```
4. 修改 `.env` 中的 `POSTGRES_PASSWORD`、`BOOTSTRAP_ADMIN_PASSWORD`、`CORS_ALLOWED_ORIGINS`。
5. 跑完整检查：`make check && make integration`。

也可以直接 `make init SLUG=acme NAME="Acme"`；`make init` 会调用同一个脚本。

生产环境把 `APP_ENV` 设为 `production`；此时启动会拒绝示例 JWT、示例管理员/数据库密码和通配 CORS。

## 技术选型

| 领域 | 选择 | 说明 |
| --- | --- | --- |
| Web 框架 | Axum 0.8 | 基于 Tokio/Hyper/Tower，中间件可组合；自定义 extractor 做鉴权很顺手 |
| 数据访问 | SQLx 0.8 | 原生 SQL、异步、迁移内嵌；本仓库用运行时查询，CI 不依赖数据库 |
| 数据库 | PostgreSQL 18 | `gen_random_uuid()`、`timestamptz`、数组聚合、功能索引 |
| 缓存/会话 | Redis 7 | 刷新令牌、权限缓存、RBAC 缓存代际 |
| 密码 | Argon2id | 每个密码独立盐值，CPU 密集操作放 `spawn_blocking` |
| 可观测性 | trace_id + JSON 日志；可选 Alloy/Loki/Grafana | 默认轻量，需要时用 profile 启用采集 |
| 部署 | Docker Compose + nginx | 2 个 API 副本，`least_conn` + Docker DNS 动态发现 |

### 工作区结构：5 个 crate

根目录是 **virtual workspace**，只放 workspace 配置、迁移和部署文件；HTTP 层也独立成 `qe-api`。

```text
.                              # virtual workspace（没有 root package）
├── Cargo.toml                 # [workspace] members + release profile
├── crates/
│   ├── api/                   # qe-api：Axum handler、config、state、bootstrap、main
│   │   └── src/
│   │       ├── api/           # 路由、handler、DTO、鉴权 extractor
│   │       ├── auth.rs        # AppState 级授权胶水
│   │       ├── bootstrap.rs   # 管理员初始化
│   │       ├── config.rs      # 环境变量
│   │       ├── error.rs       # Domain/Db/Cache/Auth -> AppError
│   │       ├── state.rs       # 共享 AppState
│   │       ├── lib.rs         # 启动流程
│   │       └── main.rs        # 二进制入口（qe-api）
│   ├── auth/                  # qe-auth：Argon2/JWT/刷新令牌/权限算法 + 存储端口
│   ├── domain/                # qe-domain：纯实体、RBAC 常量、校验、DomainError
│   ├── db/                    # qe-db：PostgreSQL 连接池/迁移/FromRow 行模型/仓储
│   │   └── src/auth_ports.rs  # 实现 PermissionSource（UserRepo）
│   └── cache/                 # qe-cache：Redis 刷新令牌与权限缓存
│       └── src/auth_ports.rs  # 实现 TokenStore / PermissionCache（Cache）
├── migrations/
├── deploy/
└── compose*.yaml / Dockerfile
```

依赖方向：

```text
qe-api ──► qe-db ──► qe-domain ──► qe-auth
   │          └────────────────────► qe-auth
   ├──► qe-cache ──────────────────► qe-auth
   └───────────────────────────────► qe-auth
```

规则：

- `qe-auth` 不依赖其他 workspace crate，也不依赖 Axum/SQLx/Redis。
- `qe-domain` 只依赖 `qe-auth`（复用权限匹配算法），不依赖 Axum/SQLx/Redis。
- `qe-db` 负责 PostgreSQL：连接池、迁移、`#[derive(FromRow)]` 行模型、SQL 与实体映射；错误是 `DbError`。
- `qe-cache` 负责 Redis：刷新令牌与权限缓存；错误是 `CacheError`。
- `qe-api` 负责 HTTP 状态码、`IntoResponse`、CORS、路由和依赖装配；不直接依赖 SQLx/Redis，只持有 `qe_db::Db` 与 `qe_cache::Cache`。`AppError` 在这一层统一映射四种下层错误。

`qe-auth` 的存储端口分别由两个适配器 crate 实现（trait 与类型分属不同 crate，只有实现方 crate 能写 impl）：

| 端口 | 作用 | 实现位置 |
| --- | --- | --- |
| `session::TokenStore` | 刷新令牌存取（`set_ex` / `GETDEL` / `del`） | `qe-cache/src/auth_ports.rs` |
| `permission::PermissionCache` | 按 RBAC 代际的权限缓存 | `qe-cache/src/auth_ports.rs` |
| `permission::PermissionSource` | 权限的事实来源 | `qe-db/src/auth_ports.rs`（`UserRepo`） |

这样 5 个 crate 都能独立测试；换数据库只改 `qe-db`，换缓存只改 `qe-cache`，换 HTTP 框架只改 `qe-api`。

### 关于 Axum vs Actix / SQLx vs ORM

- **Axum**：Tower 中间件（超时、CORS、trace、body limit）直接复用；`FromRequestParts` 写 `AuthUser` 干净；utoipa/OpenAPI 生态更统一。Actix 性能基准略高，但业务 API 的瓶颈通常在数据库，不构成理由。
- **SQLx**：RBAC 的多表 join、公告过滤用 SQL 写更透明；迁移内嵌到二进制。代价是没有 SeaORM 的实体关系便利。SQL 在 `qe-db`、缓存访问在 `qe-cache`，真要换 ORM 只改 `qe-db`。

## 快速开始（Docker）

需要 Docker 24+ 与 Compose v2。

```bash
cp .env.example .env
# 生产/长期使用务必修改：
#   APP_ENV=production
#   JWT_SECRET=$(openssl rand -hex 32)
#   POSTGRES_PASSWORD
#   BOOTSTRAP_ADMIN_PASSWORD

docker compose up -d --build
docker compose ps            # 应看到 api 的两个副本 + nginx + postgres + redis
curl -s http://localhost:8080/healthz
curl -s http://localhost:8080/readyz
```

默认引导管理员（仅当用户不存在时创建）：

```
用户名: admin
密码:   Admin@12345
```

> 这是开发默认值。部署到可被访问的环境前必须修改 `BOOTSTRAP_ADMIN_PASSWORD`，并在首次登录后改密。

登录并调用受保护接口：

```bash
TOKEN=$(curl -s http://localhost:8080/api/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{"username":"admin","password":"Admin@12345"}' | jq -r .access_token)

curl -s http://localhost:8080/api/v1/auth/me \
  -H "Authorization: Bearer $TOKEN" | jq
```

发布一条公告：

```bash
curl -s -X POST http://localhost:8080/api/v1/admin/announcements \
  -H "Authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"title":"系统上线","content":"QEStudio 后端已上线。","status":"published","is_pinned":true}' | jq
```

公开接口无需令牌：

```bash
curl -s 'http://localhost:8080/api/v1/announcements?page=1&page_size=20' | jq
```

验证负载均衡：多次请求 `/healthz`，`instance` 字段会在两个副本的 hostname 之间轮转。

```bash
for i in $(seq 1 6); do curl -s http://localhost:8080/healthz; echo; done
```

## API

统一前缀 `/api/v1`。错误响应格式：

```json
{ "error": { "code": "forbidden", "message": "缺少权限：announcement:write" } }
```

响应头包含 `X-Request-Id` 与 `X-Trace-Id`。前端/调用方上报错误时带上 `X-Trace-Id`，即可在日志中定位整条请求。

### 认证

| 方法 | 路径 | 说明 | 认证 |
| --- | --- | --- | --- |
| POST | `/auth/login` | 用户名密码登录，返回 access/refresh token + 用户信息 | 否 |
| POST | `/auth/refresh` | 用 refresh token 换新令牌（旧 token 立即失效） | 否 |
| POST | `/auth/logout` | 撤销 refresh token 并标记设备会话 | 否 |
| GET | `/auth/me` | 当前用户、角色、权限 | 是 |
| POST | `/auth/password/forgot` | 请求密码重置（开发环境把链接写日志） | 否 |
| POST | `/auth/password/reset` | 一次性令牌重置密码并撤销所有设备会话 | 否 |
| POST | `/auth/email/verification` | 请求邮箱验证 | 是 |
| POST | `/auth/email/verify` | 验证邮箱 | 否 |
| GET | `/auth/sessions` | 当前用户设备会话列表 | 是 |
| DELETE | `/auth/sessions/{id}` | 撤销指定会话 | 是 |
| DELETE | `/auth/sessions` | 撤销全部会话 | 是 |

登录会创建 `user_sessions` 记录；access token 带 `sid`，每次鉴权先查 Redis 活跃会话缓存（miss 时回退 `user_sessions` 并回填）。撤销会话会立即拒绝该 access token，而不只是阻止 refresh。迁移期内没有 `sid` 的旧 access token 仍可用，直到过期后重新登录。密码重置会撤销全部会话。

邮件：

- `EMAIL_DRIVER=log`（默认）：只把链接写入 debug 日志，适合开发；默认 `RUST_LOG=info` 看不到链接。
- `EMAIL_DRIVER=smtp`：通过 `SMTP_URL` 发送 HTML 邮件；本地用 Mailpit 验证：
  ```bash
  docker compose -f compose.yaml -f compose.mail.yaml up -d mailpit
  EMAIL_DRIVER=smtp SMTP_URL=smtp://127.0.0.1:1025 cargo run -p qe-api
  # 打开 http://127.0.0.1:8025 查看邮件（或 RUST_LOG=debug 看日志链接）
  ```
- 生产可把 `SMTP_URL` 换成企业 SMTP/API 网关；`SMTP_FROM` 必须是允许的发件人。

### 公告（公开）

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/announcements?page=&page_size=&keyword=` | 已发布公告，置顶优先 |
| GET | `/announcements/{id}` | 公告详情；未发布返回 404 |

### 公告（后台，需要权限）

| 方法 | 路径 | 所需权限 |
| --- | --- | --- |
| GET | `/admin/announcements?status=&keyword=` | `announcement:read` |
| POST | `/admin/announcements` | `announcement:write`；发布态还需 `announcement:publish` |
| PATCH | `/admin/announcements/{id}` | `announcement:write`；改状态还需 `announcement:publish` |
| DELETE | `/admin/announcements/{id}` | `announcement:delete` |

### 用户 / 角色 / 权限（后台）

| 方法 | 路径 | 所需权限 |
| --- | --- | --- |
| GET | `/admin/users?page=&page_size=&keyword=` | `user:manage` |
| POST | `/admin/users` | `user:manage` |
| PATCH | `/admin/users/{id}` | `user:manage` |
| PUT | `/admin/users/{id}/roles` | `user:manage` |
| GET | `/admin/roles` | `role:manage` 或 `user:manage` |
| POST | `/admin/roles` | `role:manage` |
| PUT | `/admin/roles/{id}/permissions` | `role:manage` |
| DELETE | `/admin/roles/{id}` | `role:manage` |
| GET | `/admin/permissions` | `role:manage` 或 `user:manage` |
| GET | `/admin/audit-logs?action=&resource_type=&page=&page_size=` | `audit:read` |

内置权限点：

```
announcement:read    announcement:write   announcement:publish
announcement:delete  user:manage          role:manage
audit:read
```

内置角色：

| 角色 | 权限 |
| --- | --- |
| `admin`（系统角色） | 全部；启动时自动补齐新权限，不可删除/改权限 |
| `editor` | `announcement:read/write/publish` |
| `viewer` | `announcement:read`（后台列表包含草稿） |

`viewer` 是后台只读角色，`announcement:read` 会看到草稿和归档内容；公开接口 `/announcements` 仍然只返回已发布公告。新建用户未指定角色时默认赋予 `viewer`。

安全约束：不能停用自己；不能停用/降级**最后一个启用中的管理员**。

### 登录与邮件限流

- 同一用户名连续失败默认 5 次（`LOGIN_MAX_ATTEMPTS`），窗口 5 分钟，锁定 15 分钟
- 同一 IP 的登录尝试默认 60 次/5 分钟（`LOGIN_IP_MAX_ATTEMPTS` / `LOGIN_IP_WINDOW_SECONDS`），成功登录也计数
- 找回密码/邮箱验证默认 5 次/15 分钟（`EMAIL_RATE_LIMIT_MAX` / `EMAIL_RATE_LIMIT_WINDOW_SECONDS`），按邮箱/用户和 IP 分别计数
- 达到阈值返回 `429 too_many_requests`，登录成功会清除失败计数和锁定
- 计数在 Redis，多副本共享；代理池/分布式攻击仍建议在 nginx/网关做全局限流

### 公告内容安全与缓存

- `content` 允许有限 HTML；写入和读取都会用 `ammonia` 清洗，移除脚本、事件属性和 `javascript:` 链接
- 前端仍应使用 DOMPurify 等库做第二道防线；服务端不清洗 Markdown 的渲染逻辑
- 公开公告列表按 `announcements:version` 代际缓存，默认 TTL 30 秒（`ANNOUNCEMENT_CACHE_SECONDS`）
- 公告创建/更新/删除时递增代际，旧缓存自然失效

### 审计日志

- 表：`audit_logs`，迁移 `0003_audit.sql`
- 记录动作：`user.create/update/set_roles`、`role.create/set_permissions/delete`、`announcement.create/update/delete`
- 记录 actor 快照、资源类型/ID、JSON 明细、时间与客户端 IP（`0005_audit_ip.sql`）
- 查询：`GET /api/v1/admin/audit-logs`，需要 `audit:read`

### OpenAPI

- Swagger UI：`/docs`
- OpenAPI 3.1 JSON：`/api-docs/openapi.json`
- 文档是手写维护的规范，避免构建时从 GitHub 下载 Swagger UI；如需自动生成可替换为 utoipa 注解 + 离线资产

## 请求追踪与日志

### trace_id / request_id

- `request_id`：边缘请求 ID。nginx 用 `$request_id` 设置 `X-Request-Id`；没有时服务生成 UUID。
- `trace_id`：W3C Trace Context 中的 trace-id（32 位十六进制），当前同时作为请求级追踪 ID。优先从 `traceparent` 解析，其次读 `X-Trace-Id`，都没有就生成。
- 两者都会写入 `http_request` span，因此该请求内的所有日志自动带 `trace_id` / `request_id`；同时通过响应头 `X-Trace-Id` / `X-Request-Id` 返回，并在 CORS 中暴露给浏览器。

```bash
curl -i -H 'traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01' \
  http://localhost:8080/healthz
```

响应头会带回 `x-trace-id: 4bf92f3577b34da6a3ce929d0e0e4736`。前端上报错误时把这个头一起带上，就能直接按 trace_id 查日志。

当前实现是“请求级 trace ID + 日志关联”，没有导出 span。等出现服务间调用时，再接 `tracing-opentelemetry` + OTLP Collector + Tempo/Jaeger，即可得到跨服务链路；`traceparent` 的解析入口已经留好。

### 日志

- 应用只写 stdout，容器日志由 Docker `json-file` 驱动保存；compose 已限制 `max-size=10m`、`max-file=3`，避免无限增长。
- 容器内固定 `LOG_FORMAT=json`（`tracing-subscriber` JSON），本地 `cargo run -p qe-api` 仍可用 `.env` 的 `LOG_FORMAT=pretty`。
- JSON 日志里的 `span.trace_id`、`span.request_id` 可直接用于检索。
- 应用日志不是审计日志。谁改了哪个公告/角色/用户，需要单独建 `audit_logs` 表，不要只依赖日志。

### 日志采集方案

当前 compose 默认只保留 Docker 本地日志，没有采集器。生产建议按规模选：

| 方案 | 组成 | 优点 | 代价 / 适用 |
| --- | --- | --- | --- |
| Grafana 栈（推荐） | Alloy（采集）→ Loki（存储）→ Grafana（查询） | 轻量、按标签检索、和指标/追踪共用 Grafana；容器环境成熟 | 全文检索弱于 ES；适合中小规模 |
| ELK / EFK | Fluent Bit/Logstash → Elasticsearch → Kibana | 全文检索强、生态最成熟 | JVM 内存与运维成本高；适合日志量大、检索复杂 |
| ClickHouse 系 | OTel Collector → ClickHouse（SigNoz/Uptrace） | logs/traces/metrics 一体，APM 体验好 | 组件较重；想一步到位做可观测性时选 |
| 云日志 | 阿里 SLS / 腾讯 CLS / 华为 LTS | 免运维、告警/归档成熟 | 有费用和厂商锁定；已在云上时最省事 |

这个项目建议用 **Grafana Alloy + Loki + Grafana**，保留 7-14 天；后面要链路追踪，再加 Tempo/OTel Collector。若部署在阿里云/腾讯云，直接用 SLS/CLS 更省运维；不建议在这个体量上先上 ELK。

### 可选的 Grafana 可观测性栈

仓库已经提供 [compose.observability.yaml](compose.observability.yaml)，**默认不启动**，只有显式带 `--profile observability` 才生效：

```bash
# 启动：base 栈 + Loki / Alloy / Grafana
docker compose -f compose.yaml -f compose.observability.yaml --profile observability up -d

# 只停可观测性组件，不影响 API/PG/Redis/nginx
docker compose -f compose.yaml -f compose.observability.yaml --profile observability stop loki alloy grafana
```

### OTLP trace / logs 导出

API 已接入 `tracing-opentelemetry` + OTLP/HTTP。默认关闭；设置：

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318
OTEL_SERVICE_NAME=qestudio-backend
# 可选：同时导出 OTLP logs；默认仍保留 stdout JSON
OTEL_LOGS_ENABLED=true
```

启用后，`http_request` span 和 tracing 日志会导出到 OTLP collector。没有 collector 时不要设置该变量；日志仍按 JSON 输出，由 Alloy/Loki 或外部采集器收集。同时开启 stdout 和 OTLP logs 会造成重复采集，按部署环境二选一。

访问地址（默认只绑定 `127.0.0.1`）：

| 组件 | 地址 | 说明 |
| --- | --- | --- |
| Grafana | http://127.0.0.1:3000 | 默认 `admin/admin`，用 `GRAFANA_ADMIN_PASSWORD` 覆盖 |
| Loki | http://127.0.0.1:3100 | 日志存储/查询 API |
| Alloy | http://127.0.0.1:12345 | 采集器 UI/调试 |

组成：Alloy 通过 Docker API 读取本 compose 项目（project name `qestudio`）的容器 stdout → Loki（文件系统存储，7 天 retention）→ Grafana 已自动挂好 Loki 数据源。

Grafana Explore 里可以直接查：

```logql
{service="api"}
{service="api"} |= "trace_id" |= "4bf92f3577b34da6a3ce929d0e0e4736"
{service="api"} | json | level="ERROR"
```

`service` 来自 compose service 名，`container` 是容器名，`stream` 是 stdout/stderr，`job="docker"`。

注意：

- 端口默认只绑 `127.0.0.1`；远程访问请用 SSH 隧道，或自行改 compose 的端口绑定。
- Alloy 挂载 `/var/run/docker.sock:ro`，这等同于宿主机 root 权限，只在可信环境启用。
- 默认 Grafana 管理员密码是 `admin`，启用前务必在 `.env` 设置 `GRAFANA_ADMIN_PASSWORD`。
- 数据卷为 `qestudio_lokidata` / `qestudio_alloydata` / `qestudio_grafanadata`；`down -v` 会删除。
- Alloy 的采集过滤条件写死为 `com.docker.compose.project=qestudio`；如果改 compose 项目名，需要同步改 [deploy/observability/alloy.alloy](deploy/observability/alloy.alloy)。
- 默认 `docker compose up -d` 不会启动这些组件；只有带 `--profile observability` 且显式引用 `compose.observability.yaml` 才会启用。
- Alloy 自身日志不采集（避免 Loki 故障时形成采集回环）；排查 Alloy 用 `docker logs qestudio-alloy-1`。

## 配置

全部来自环境变量，本地由 `.env` 加载。

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `BIND_ADDR` | `0.0.0.0:8080` | 监听地址 |
| `DATABASE_URL` | 必填 | PostgreSQL 连接串 |
| `DB_MAX_CONNECTIONS` | `10` | 每副本连接池上限 |
| `REDIS_URL` | `redis://127.0.0.1:6379` | Redis 连接串 |
| `JWT_SECRET` | 必填，≥32 字符 | HS256 密钥；更换会使所有 access token 失效 |
| `ACCESS_TOKEN_TTL_SECONDS` | `900` | 访问令牌有效期 |
| `REFRESH_TOKEN_TTL_SECONDS` | `604800` | 刷新令牌有效期，必须大于 access TTL |
| `CORS_ALLOWED_ORIGINS` | `http://localhost:5173` | 逗号分隔；`*` 表示任意来源 |
| `RUN_MIGRATIONS` | `true` | 启动时执行迁移（多副本并发安全） |
| `BOOTSTRAP_ADMIN_USERNAME` | 空 | 设置后尝试创建引导管理员 |
| `BOOTSTRAP_ADMIN_PASSWORD` | 空 | 引导管理员密码 |
| `BOOTSTRAP_ADMIN_DISPLAY_NAME` | `Administrator` | 显示名 |
| `APP_ENV` | `development` | `production` 时拒绝示例密钥/通配 CORS |
| `REQUEST_TIMEOUT_SECONDS` | `30` | 单请求超时 |
| `MAX_BODY_BYTES` | `2097152` | 请求体上限（2 MiB） |
| `LOGIN_MAX_ATTEMPTS` | `5` | 登录失败次数阈值 |
| `LOGIN_WINDOW_SECONDS` | `300` | 失败计数窗口 |
| `LOGIN_LOCK_SECONDS` | `900` | 触发阈值后的锁定时长 |
| `LOGIN_IP_MAX_ATTEMPTS` | `60` | 同一 IP 登录窗口内最大尝试次数 |
| `LOGIN_IP_WINDOW_SECONDS` | `300` | 登录 IP 计数窗口 |
| `EMAIL_RATE_LIMIT_MAX` | `5` | 找回密码/邮箱验证窗口内最大次数 |
| `EMAIL_RATE_LIMIT_WINDOW_SECONDS` | `900` | 邮件类接口计数窗口 |
| `ANNOUNCEMENT_CACHE_SECONDS` | `30` | 公开公告列表缓存 TTL |
| `PUBLIC_BASE_URL` | `http://localhost:5173` | 邮件链接前缀 |
| `PASSWORD_RESET_TTL_SECONDS` | `1800` | 密码重置令牌有效期 |
| `EMAIL_VERIFICATION_TTL_SECONDS` | `86400` | 邮箱验证令牌有效期 |
| `EMAIL_DRIVER` | `log` | `log`（开发）或 `smtp` |
| `SMTP_URL` | `smtp://127.0.0.1:1025` | SMTP 连接串（Mailpit 默认） |
| `SMTP_FROM` | `QEStudio <no-reply@qestudio.local>` | 发件人 |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | 空 | 设置后启用 OTLP/HTTP trace 导出 |
| `OTEL_SERVICE_NAME` | `qestudio-backend` | OTel service.name |
| `OTEL_LOGS_ENABLED` | `false` | 同时导出 OTLP logs（默认仅 stdout） |
| `RUST_LOG` | `info,sqlx=warn` | 日志过滤 |
| `LOG_FORMAT` | `pretty` | 本地日志格式；compose 容器内固定为 `json` |

## 本地开发

```bash
cp .env.example .env
make dev-up                           # 只启动 PG/Redis，并提示下一步
cargo run -p qe-api                    # 启动 API（自动迁移 + 创建管理员）
cargo test --workspace                # 单元测试（5 个 crate）
make integration                       # 真实 PostgreSQL + Redis 集成测试
make mail-up                           # 可选：Mailpit，http://127.0.0.1:8025
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

`.env` 里的 `DATABASE_URL` 指向 `localhost`；compose 内的 API 会被覆盖为 `postgres:5432`。

### 迁移

- 应用启动时自动执行（`sqlx::migrate!` 已把 `migrations/` 编进二进制，镜像里不需要额外文件）。
- 多副本同时启动时，sqlx 用 PostgreSQL advisory lock 串行化迁移。
- 也可用 `sqlx-cli` 手动执行：`sqlx migrate run`（需要先 `cargo install sqlx-cli`）。
- 不要修改已执行过的迁移文件；新增变更请加 `0003_xxx.sql`。

## Docker 与负载均衡

`compose.yaml` 默认：

- `postgres`：PostgreSQL 18，仅绑定 `127.0.0.1:5432`，便于本机调试；数据卷挂在 `/var/lib/postgresql`（18 镜像的 PGDATA 布局）；
- `redis`：AOF 持久化；
- `api`：`deploy.replicas: 2`，不映射宿主端口，仅 `expose 8080`；
- `nginx`：宿主 `${HTTP_PORT}`（默认 8080）→ 容器 80，`least_conn` 转发到 `api`。

镜像版本可通过 `POSTGRES_IMAGE` / `REDIS_IMAGE` / `NGINX_IMAGE` 环境变量覆盖；Dockerfile 的构建基镜像也可通过 `RUST_IMAGE` / `RUNTIME_IMAGE` 覆盖，方便离线或内网镜像站。

nginx 使用 `resolver 127.0.0.11` + `server api:8080 resolve`，每 5 秒重新解析 Docker DNS，因此 `docker compose up -d --scale api=3` 后无需重启 nginx。

生产环境用 CI 产出的镜像：

```bash
export BACKEND_IMAGE=registry.example.com/team/backend:latest
export POSTGRES_PASSWORD="$(openssl rand -hex 16)"
export BOOTSTRAP_ADMIN_PASSWORD="$(openssl rand -base64 24)"
export JWT_SECRET="$(openssl rand -hex 32)"
docker compose -f compose.yaml -f compose.prod.yaml pull
docker compose -f compose.yaml -f compose.prod.yaml up -d
```

`compose.prod.yaml` 会移除本地构建、去掉 PostgreSQL 的宿主端口。

Kubernetes 部署请使用 `deploy/k8s/deployment.yaml`：`readinessProbe` + `preStop` +
PDB 做主动摘流，说明见 [docs/K8S.md](docs/K8S.md)。开源 nginx 的 `max_fails`
仍只是被动检查，不能替代 readiness。

### HTTPS

当前 nginx 只监听 80。生产建议在它前面放云负载均衡/Ingress 做 TLS 终止，或给 nginx 挂证书并加 443 server 块；应用只信任 `X-Forwarded-Proto`，不直接处理证书。

## GitHub Actions / Gitea Actions

GitHub 工作流：`.github/workflows/ci.yml`（镜像推送到 GHCR）；Gitea 工作流：`.gitea/workflows/ci.yml`（镜像推送到 Gitea Registry）。两边都包含 test、integration、image 三个 job，image 依赖 test + integration。

Gitea 工作流步骤：

1. `test` job：`cargo fmt --all --check` → `cargo clippy --workspace --all-targets -D warnings` → `cargo test --workspace`。
2. `integration` job：启动 PostgreSQL/Redis，执行 `scripts/test-integration.sh`（含 HTTP 登录/刷新/RBAC/会话撤销测试）。
3. `image` job（push/tag 触发，依赖前两个 job）：构建镜像并推送，标签为 `sha-<短 commit>` 与 `latest`（默认分支）。

在 Gitea 仓库 **Settings → Actions** 配置：

| 类型 | 名称 | 示例 |
| --- | --- | --- |
| Variable | `REGISTRY` | `git.example.com` |
| Secret | `REGISTRY_USERNAME` | Gitea 用户名或 token 用户名 |
| Secret | `REGISTRY_PASSWORD` | 具有 package 写权限的 token |

Runner 必须能访问 Docker daemon（挂载 `/var/run/docker.sock` 或使用 DinD）。没有 Docker 权限时，`test` job 仍可运行，把 `image` job 去掉即可。

Gitea 镜像名使用 `${{ vars.REGISTRY }}/${{ github.repository }}`，请确保仓库路径为小写（Docker tag 不允许大写）。GitHub 侧默认推送到 `ghcr.io/<owner>/<repo>`，使用内置 `GITHUB_TOKEN`，只需在仓库 Settings → Actions 允许写 packages。

服务器更新：

```bash
docker compose -f compose.yaml -f compose.prod.yaml pull api
docker compose -f compose.yaml -f compose.prod.yaml up -d api
```

## 设计说明

- **令牌**：access 为 HS256 JWT（默认 15 分钟），refresh 为 32 字节随机数，Redis 只存 SHA-256 摘要，`GETDEL` 原子消费并轮换。登出/刷新只撤销 refresh；access 在过期前仍有效（无状态 JWT 的取舍）。
- **权限缓存**：Redis key 带 RBAC 代际（`rbac:v{n}:user:{id}:perms`）。改角色/权限/停用用户时 `INCR` 代际，旧缓存自然失效；TTL 5 分钟兜底。Redis 故障时自动回退数据库。
- **多副本**：所有会话/缓存状态在 Redis，API 无本地会话，任意副本可处理任意请求。
- **迁移与引导**：启动自动迁移；`admin` 角色每次启动补齐全部权限；引导管理员只在不存在时创建，不会覆盖已有密码。

### 已知限制 / 后续可做

- nginx 开源版只有被动健康检查（`max_fails`），Docker healthcheck 不会自动把副本从 DNS 移除；生产建议用 K8s readiness + preStop + PDB，清单在 `deploy/k8s/`，说明见 `docs/K8S.md`。
- 公告写入/读取已做 HTML 清洗；前端渲染 Markdown 时仍需使用安全的 Markdown 渲染器，不能直接 `innerHTML`。
- 登录失败限制按用户名计数；生产仍建议在 nginx/网关按 IP 做额外限流，防止轮换用户名。
- 审计日志已落库并可按 `audit:read` 查询；客户端 IP 来自 `X-Real-IP`/`X-Forwarded-For`，直接暴露 API 时需要网关覆盖这两个头。审计归档/防篡改链未做。
- OTLP trace 导出已接入；日志默认走 JSON stdout → Alloy/Loki。设置 `OTEL_LOGS_ENABLED=true` 可同时导出 OTLP logs；两者同时开启会重复采集，按部署环境二选一。
- 密码找回、邮箱验证、多设备会话已实现；`EMAIL_DRIVER=smtp` 时使用 SMTP（本地可用 `compose.mail.yaml` 的 Mailpit）。`log` 仅用于开发，且默认不把令牌写进 info 日志。
- 集成测试已提供 `scripts/test-integration.sh`，覆盖仓储、缓存、登录/刷新/RBAC/会话撤销和安全响应头。
- `announcements` 是示例业务模块，用于演示 CRUD + RBAC + 缓存；不需要时删除对应 handler、repo、路由和权限即可。
- `/docs` 的 Swagger UI 从 unpkg CDN 加载资源，离线/内网部署需要自托管静态资源或改用其他文档页。
- K8s 清单是 starter：Ingress/TLS、NetworkPolicy、HPA、外部 Secret 需要按环境补。

## 数据库抖动与故障处理

当前实现按“快速失败 + 状态语义正确 + 可恢复”处理：

- 连接池：`min_connections=1`、`acquire_timeout=3s`、`idle_timeout=10m`、`max_lifetime=30m`，取连接前用 `test_before_acquire` 探活；数据库重启后会丢弃坏连接并重新建立。
- 每条连接设置 `statement_timeout=10s`、`lock_timeout=3s`、`idle_in_transaction_session_timeout=15s`；慢查询、锁等待和事务悬挂不会无限占用连接。
- 错误分类：`PoolTimedOut`、`PoolClosed`、`Io`、SQLSTATE `08*` / `57P*`，以及锁超时/死锁/连接数打满（`55P03` / `40001` / `40P01` / `53300` / `57014` / `25P03`）映射为 `DbError::Unavailable` → HTTP `503 service_unavailable`；唯一键/外键/检查约束仍是 409/400。nginx 的 `proxy_next_upstream http_503` 会对幂等请求尝试其他副本。
- `/readyz` 在数据库或 Redis 不可用时返回 503；`/healthz` 只表示进程存活，不因数据库抖动触发容器重启。
- 启动阶段迁移/引导管理员失败会让进程退出，由 compose `restart` 或编排系统重启；多副本迁移有 advisory lock。
- 写请求不自动重试：POST/PATCH 可能已经被数据库提交但响应丢失，盲目重试会重复写入；需要幂等键或业务去重。
- Redis 里的权限缓存命中时，数据库短时不可用仍可完成权限判断；但登录、公告列表、用户查询仍依赖 PostgreSQL。
- 连接数 = 副本数 × `DB_MAX_CONNECTIONS`（默认 10）；副本多或单副本连接多时加 PgBouncer；读写分离则引入只读副本和独立连接串。
- 建议监控：503 比例、`PoolTimedOut` 日志、PostgreSQL 连接数/锁等待/慢查询、请求 P99；告警优先看 `/readyz` 和 5xx。
- 大表 DDL 请用独立迁移 job，并把应用设为 `RUN_MIGRATIONS=false`，避免 10s `statement_timeout` 中断迁移。

公开公告列表有 30 秒 Redis 代际缓存；缓存未命中且数据库不可用时仍会返回 503。若需要数据库故障期间持续可读，可延长 TTL 并做启动预热，代价是短暂旧数据。

## 排障

| 现象 | 排查 |
| --- | --- |
| 启动报 `JWT_SECRET` | `.env` 未配置或长度不足 32 |
| `readyz` 返回 503 | `docker compose logs postgres redis api`；确认 PG/Redis healthy |
| nginx 502 | API 尚未 ready；看 `docker compose ps` 与 `docker compose logs api` |
| 登录 401 | 用户名/密码错误；确认引导管理员是否已创建（日志 `引导管理员`） |
| 403 缺少权限 | 检查用户角色；`GET /auth/me` 看 permissions |
| CORS 失败 | `CORS_ALLOWED_ORIGINS` 是否包含前端 origin，需带协议和端口 |
| 构建镜像很慢 | 首次构建要编译全部依赖；CI 用 registry cache，本地可配 crates 镜像 |

## License

MIT © 2026 QEStudio. See [LICENSE](LICENSE).
