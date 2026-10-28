# QEStudio Backend

基于 **Rust + Axum + PostgreSQL + Redis** 的 Web API 服务，内置：

- 用户名/密码登录，JWT 访问令牌 + Redis 一次性刷新令牌
- RBAC 权限模型（用户 → 角色 → 权限），管理员可在运行时改角色/权限
- 公告管理（草稿 / 发布 / 归档 / 置顶 / 分页 / 搜索）与公开公告接口
- Docker 多阶段构建，nginx 反向代理 + 多副本负载均衡
- Gitea Actions：格式检查、Clippy、单测、镜像构建并推送到 Gitea Registry

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

### 工作区结构：`crates/auth` + 其余单 crate

认证是唯一需要编译期边界的地方：它不该反过来依赖 Axum、PostgreSQL 或业务实体，否则以后想独立部署时改不动。所以它单独成 crate（包名 `qe-auth`），其余仍是单 crate + 模块分层。

```text
backend/
├── crates/auth/          # 认证原语，不认识 AppState
│   └── src/
│       ├── password.rs   Argon2id 哈希 / 校验
│       ├── jwt.rs        HS256 访问令牌
│       ├── session.rs    刷新令牌 + TokenStore 端口
│       ├── permission.rs 权限判定 + 缓存端口
│       └── error.rs      AuthError
├── src/
│   ├── auth.rs           适配层：把 Cache / UserRepo 接到上面的端口
│   ├── api/              路由、handler、DTO、鉴权 extractor
│   ├── domain/           实体、RBAC 权限点与角色、输入校验
│   ├── infra/            PostgreSQL 仓储、Redis 缓存
│   ├── bootstrap.rs      管理员初始化
│   ├── config.rs         环境变量
│   ├── error.rs          统一错误 → HTTP（含 AuthError 映射）
│   ├── state.rs          共享 AppState
│   ├── lib.rs            启动流程
│   └── main.rs           二进制入口
```

边界靠三个端口维持，`qe-auth` 只认 trait 不认实现：

| 端口 | 作用 | 后端实现 |
| --- | --- | --- |
| `session::TokenStore` | 刷新令牌存取（`set_ex` / `GETDEL` / `del`） | `Cache`（Redis） |
| `permission::PermissionCache` | 按 RBAC 代际的权限缓存 | `Cache`（Redis） |
| `permission::PermissionSource` | 权限的事实来源 | `UserRepo`（PostgreSQL） |

因此 `qe-auth` 不依赖 axum：`AuthUser` extractor、`IntoResponse` 和状态码留在后端，由 `AppError` 统一。真要把它拆成独立服务时，替换三个端口的实现即可，内部逻辑不用重写。

`domain` 仍然只依赖 `error`，以后要拆 `crates/domain`、`crates/infra` 同样是平移，不需要推翻结构。

### 关于 Axum vs Actix / SQLx vs ORM

- **Axum**：Tower 中间件（超时、CORS、trace、body limit）直接复用；`FromRequestParts` 写 `AuthUser` 干净；utoipa/OpenAPI 生态更统一。Actix 性能基准略高，但业务 API 的瓶颈通常在数据库，不构成理由。
- **SQLx**：RBAC 的多表 join、公告过滤用 SQL 写更透明；迁移内嵌到二进制。代价是没有 SeaORM 的实体关系便利。数据访问集中在 `infra/`，真要换 ORM 只改这一层。

## 快速开始（Docker）

需要 Docker 24+ 与 Compose v2。

```bash
cp .env.example .env
# 生产/长期使用务必修改：
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
| POST | `/auth/logout` | 撤销 refresh token | 否 |
| GET | `/auth/me` | 当前用户、角色、权限 | 是 |

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

内置权限点：

```
announcement:read    announcement:write   announcement:publish
announcement:delete  user:manage          role:manage
```

内置角色：

| 角色 | 权限 |
| --- | --- |
| `admin`（系统角色） | 全部；启动时自动补齐新权限，不可删除/改权限 |
| `editor` | `announcement:read/write/publish` |
| `viewer` | `announcement:read`（后台列表包含草稿） |

`viewer` 是后台只读角色，`announcement:read` 会看到草稿和归档内容；公开接口 `/announcements` 仍然只返回已发布公告。新建用户未指定角色时默认赋予 `viewer`。

安全约束：不能停用自己；不能停用/降级**最后一个启用中的管理员**。

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
- 容器内固定 `LOG_FORMAT=json`（`tracing-subscriber` JSON），本地 `cargo run` 仍可用 `.env` 的 `LOG_FORMAT=pretty`。
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
| `REQUEST_TIMEOUT_SECONDS` | `30` | 单请求超时 |
| `MAX_BODY_BYTES` | `2097152` | 请求体上限（2 MiB） |
| `RUST_LOG` | `info,sqlx=warn` | 日志过滤 |
| `LOG_FORMAT` | `pretty` | 本地日志格式；compose 容器内固定为 `json` |

## 本地开发

```bash
cp .env.example .env
docker compose up -d postgres redis   # 只起依赖
cargo run                             # 启动 API（自动迁移 + 创建管理员）
cargo test --workspace                # 单元测试（含 crates/auth）
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

生产环境用 Gitea CI 产出的镜像：

```bash
export BACKEND_IMAGE=registry.example.com/team/backend:latest
docker compose -f compose.yaml -f compose.prod.yaml pull
docker compose -f compose.yaml -f compose.prod.yaml up -d
```

`compose.prod.yaml` 会移除本地构建、去掉 PostgreSQL 的宿主端口。

### HTTPS

当前 nginx 只监听 80。生产建议在它前面放云负载均衡/Ingress 做 TLS 终止，或给 nginx 挂证书并加 443 server 块；应用只信任 `X-Forwarded-Proto`，不直接处理证书。

## Gitea Actions

工作流：`.gitea/workflows/ci.yml`

1. `test` job：`cargo fmt --all --check` → `cargo clippy --workspace -D warnings` → `cargo test --workspace`。
2. `image` job（push/tag 触发）：构建镜像并推送，标签为 `sha-<短 commit>` 与 `latest`（默认分支）。

在 Gitea 仓库 **Settings → Actions** 配置：

| 类型 | 名称 | 示例 |
| --- | --- | --- |
| Variable | `REGISTRY` | `git.example.com` |
| Secret | `REGISTRY_USERNAME` | Gitea 用户名或 token 用户名 |
| Secret | `REGISTRY_PASSWORD` | 具有 package 写权限的 token |

Runner 必须能访问 Docker daemon（挂载 `/var/run/docker.sock` 或使用 DinD）。没有 Docker 权限时，`test` job 仍可运行，把 `image` job 去掉即可。

镜像名使用 `${{ vars.REGISTRY }}/${{ github.repository }}`，请确保仓库路径为小写（Docker tag 不允许大写）。

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

- nginx 开源版只有被动健康检查（`max_fails`），Docker healthcheck 不会自动把副本从 DNS 移除；生产可换 K8s/云 LB 做主动摘流。
- 公告 `content` 按原样存储，前端渲染 HTML/Markdown 时必须做 XSS 过滤（当前接口不负责清洗）。
- 登录没有失败次数限制，建议在 nginx 或 Redis 加限流/锁定。
- 没有审计日志表（谁改了哪个用户/角色/公告）。
- 没有接入 OpenTelemetry span 导出和日志采集器；当前只有请求级 trace_id 与 JSON 日志。
- 没有开放 API 文档（可接 utoipa + Swagger UI）。
- 没有密码找回、邮箱验证、多设备会话管理。
- 公告列表未缓存；如需高读吞吐可按 RBAC 同样的代际思路加缓存。
- 集成测试需要真实 PG/Redis，目前只有单元测试。

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
