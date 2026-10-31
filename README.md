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

## 课表与事件

课表能力按「区域 → 房间 → 课表 → 排课实例 → 触发规则 → 事件/投递」组织，当前只实现**房间节点**。
设计契约见 [`docs/schedule-design.md`](docs/schedule-design.md)，数据库迁移为 `migrations/0009_schedule.sql`。

时间约定：所有具体时间点（实例 `start_at` / `end_at`、事件 `triggered_at`）一律以 **UTC** 存储并返回 RFC3339；
展示时按**区域（area）的 IANA 时区**转换成本地时间。房间/课表继承区域时区，周模板与单日覆盖的 `TIME`
是本地钟面时间（由所属区域时区解释）。

### 数据模型

```
areas ──< rooms ──< schedules ──< schedule_weekly_slots（周模板）
                  │           ├──< schedule_exceptions ──< schedule_exception_slots（单日覆盖）
                  │           └──< schedule_occurrences（展开后的排课实例）
                  └──< schedule_event_rules（触发规则，按房间）

schedule_events（事件，event_key 唯一保证幂等）──< schedule_event_deliveries（每个投递目标一行）
```

- **区域（area）**：楼栋 / 楼层 / 片区的两级组织；`parent_id` 与 `metadata` 为总管理节点 / 子管理节点预留。
  `timezone` 是该地区的 IANA 时区（缺省 `Asia/Shanghai`），也是整个课表链路唯一的时区来源。
- **房间（room）**：调度最小单位，可绑定一个设备槽位 `device_slot`（对应 `device_enrollment.device_slot`，
  唯一），并可为 webhook 投递配置默认 `webhook_url`；时区只读继承自所属区域（响应字段 `timezone`）。
- **课表（schedule）**：挂在房间上，包含生效日期范围、默认周循环与单日覆盖；同一房间可有多份课表
  （通常按学期/周期区分）；时区同样只读继承自区域。
- **周模板（weekly slot）**：ISO 星期（周一=1..周日=7）+ 本地时段 `"HH:MM"`；`room_id` 非空时表示调课到其他房间。
- **单日覆盖（exception）**：某天 `cancel`（停课）或用 `replace` 时段替换当天周模板。
- **排课实例（occurrence）**：周模板 × 日期（或单日覆盖）展开后的具体一节课，落库物化，供事件引擎与查询使用。
  `start_at` / `end_at` 存 UTC，响应同时给出 `timezone`、`start_local`、`end_local` 便于直接展示。
  只重写**未来**（`start_at > now()`）的实例，已过去的实例与事件引用保持不变。
- **触发规则（event rule）**：按房间配置，支持 `occurrence_start`、`occurrence_end`、`occurrence_start_before`、
  `daily_time`、`room_occupied`、`room_idle`；`config` 指定投递目标与 webhook/下行参数。
- **事件与投递（event / delivery）**：事件插入幂等（`event_key` 唯一），同事务写入 `pubsub` / `downlink` /
  `webhook` 三类投递行；投递失败按 `next_attempt_at` 退避重试。事件保存生成时的区域时区快照 `timezone`，
  响应另给出 `triggered_at_local`。

### 时区与本地时间

- `areas.timezone` 是唯一的时区来源，取值为 IANA 时区名（如 `Asia/Shanghai`、`Europe/Berlin`），
  创建时缺省 `Asia/Shanghai`；房间与课表不再单独保存 UTC 偏移，响应中的 `timezone` 为只读继承值。
- 所有时间点以 UTC 存储（PostgreSQL `TIMESTAMPTZ`），API 返回的 `start_at` / `end_at` / `triggered_at`
  是 UTC RFC3339（`Z` 结尾）。便于展示的本地时间由后端一并返回：
  - 排课实例：`timezone` + `start_local` / `end_local`（如 `2026-11-02T08:00:00+08:00`）；
  - 事件：`timezone`（区域时区快照）+ `triggered_at_local`。
- 周模板 / 单日覆盖的 `start_time` / `end_time` 与触发规则的 `at_time` 是本地钟面时间 `"HH:MM"`，
  按所属区域时区解释；DST 跳变中不存在的时段返回 400，重复时段取较早的一次。
- 修改区域 `timezone` 成功后，服务端会按新时区重建该区域下所有课表的未来排课实例；
  重建失败时区域已更新、接口返回 500 并记录日志，可稍后手动触发重建。
- `GET /schedules/{id}/occurrences`、`GET /rooms/{id}/occurrences` 的默认日期区间按课表/房间所属区域时区计算“今天”。

### 课表 API

全部挂在 `/api/v1/admin` 下，需要 Bearer 认证与对应权限；分页参数为 `page` / `page_size`（`1..=100`）。

| 方法 | 路径 | 所需权限 | 说明 |
| --- | --- | --- | --- |
| GET | `/areas?page=&page_size=&keyword=&include_inactive=` | `area:read` | 区域分页 |
| POST | `/areas` | `area:manage` | 创建区域（可选 `timezone`，缺省 `Asia/Shanghai`） |
| GET | `/areas/{id}` | `area:read` | 区域详情 |
| PATCH | `/areas/{id}` | `area:manage` | 局部更新；`parent_id` / `metadata` 可传 `null`；`timezone` 变更后重建区域实例 |
| DELETE | `/areas/{id}` | `area:manage` | 删除区域；仍有房间时 409 |
| GET | `/rooms?area_id=&page=&page_size=&keyword=&include_inactive=` | `room:read` | 房间分页 |
| POST | `/rooms` | `room:manage` | 创建房间 |
| GET | `/rooms/{id}` | `room:read` | 房间详情 |
| PATCH | `/rooms/{id}` | `room:manage` | 局部更新；`device_slot` / `webhook_url` 可传 `null` 清空 |
| DELETE | `/rooms/{id}` | `room:manage` | 删除房间，级联课表、覆盖、实例与触发规则 |
| GET | `/schedules?room_id=&page=&page_size=` | `schedule:read` | 课表分页 |
| POST | `/schedules` | `schedule:manage` | 创建课表 |
| GET | `/schedules/{id}` | `schedule:read` | 课表详情 |
| PATCH | `/schedules/{id}` | `schedule:manage` | 局部更新；成功后自动重建未来实例 |
| DELETE | `/schedules/{id}` | `schedule:manage` | 删除课表并级联实例 |
| GET | `/schedules/{id}/weekly-slots` | `schedule:read` | 周模板 |
| PUT | `/schedules/{id}/weekly-slots` | `schedule:manage` | 全量替换周模板（body `{slots:[...]}`）；成功后重建 |
| GET | `/schedules/{id}/exceptions?from=&to=` | `schedule:read` | 单日覆盖列表 |
| PUT | `/schedules/{id}/exceptions/{date}` | `schedule:manage` | upsert 覆盖（body `{kind:"cancel"\|"replace",note?,slots?:[...]}`）；成功后重建 |
| DELETE | `/schedules/{id}/exceptions/{date}` | `schedule:manage` | 删除覆盖并重建 |
| GET | `/schedules/{id}/occurrences?from=&to=` | `schedule:read` | 排课实例（UTC + `timezone` / `start_local` / `end_local`；默认按区域时区今天起 7 天，跨度 ≤366 天） |
| GET | `/rooms/{id}/occurrences?from=&to=` | `schedule:read` | 房间维度排课实例（同上，按房间继承时区计算默认区间） |
| POST | `/schedules/{id}/rebuild` | `schedule:manage` | 手动重建未来实例，返回 `{"occurrences":n}` |
| GET | `/rooms/{id}/event-rules` | `schedule:read` | 房间触发规则 |
| PUT | `/rooms/{id}/event-rules` | `schedule:manage` | 全量替换触发规则（body `{rules:[...]}`） |
| GET | `/schedule-events?room_id=&kind=&from=&to=&page=&page_size=` | `schedule:event:read` | 事件分页（`triggered_at` 倒序；含 `timezone` / `triggered_at_local`） |
| GET | `/schedule-events/{id}` | `schedule:event:read` | 事件 + 投递明细 `{event,deliveries}` |
| POST | `/schedule-events/{id}/replay` | `schedule:event:manage` | 全部投递重置为 `pending`，返回 `{"reset":n}` |
| POST | `/schedule-events/{id}/deliveries/{delivery_id}/retry` | `schedule:event:manage` | 单条投递立即重试，204 |

请求体可选字段语义：**缺失 = 不修改**；对可空字段（`parent_id`、`device_slot`、`webhook_url`、`metadata`）
传 `null` 表示清空。创建/更新区域可传 `timezone`（IANA 时区名）；房间、课表请求体不再接受 UTC 偏移，
时区由所属区域继承。`TIME` 字段统一为 `"HH:MM"`（也接受 `"HH:MM:SS"`），日期为 `"YYYY-MM-DD"`。

写操作成功后写审计日志，动作名：`area.create/update/delete`、`room.create/update/delete`、
`schedule.create/update/delete`、`schedule.set_weekly_slots`、`schedule.set_exception`、
`schedule.delete_exception`、`room.set_event_rules`、`schedule_event.replay`、`schedule_event.retry`。

`PATCH /schedules/{id}`、`PUT weekly-slots`、`PUT exception`、`DELETE exception` 在写成功后调用
`rebuild_occurrences` 重建未来实例；重建在事务外进行，失败时写操作已提交、接口返回 500 并记录日志，
可稍后用 `POST /schedules/{id}/rebuild` 或后台重建任务补齐。

### 权限点

```
area:read    area:manage    room:read    room:manage
schedule:read    schedule:manage    schedule:event:read    schedule:event:manage
```

- `admin` 角色自动拥有全部权限（迁移与 bootstrap 都会补齐）；
- `viewer` / `editor` 默认获得 4 个只读权限（`area:read`、`room:read`、`schedule:read`、`schedule:event:read`）；
- 触发规则的读权限为 `schedule:read`，写权限为 `schedule:manage`（规则挂在房间下）。

### 触发与投递语义

- 引擎默认每 `SCHEDULE_TICK_SECONDS` 秒扫描一次；窗口为 `[now - lookback, now]`，
  `lookback = max(60s, tick × 4)`。事件 `event_key` 唯一，重叠窗口天然幂等。
- 触发条件：
  - `occurrence_start` / `occurrence_end`：实例开始 / 结束时刻落在窗口内；
  - `occurrence_start_before`：`start_at - offset_minutes` 落在窗口内（`offset_minutes > 0`）；
  - `daily_time`：按房间所属区域时区（IANA），本地 `at_time` 落在窗口内（跨午夜时两天都判断，DST 正确）；
  - `room_occupied` / `room_idle`：实例开始 / 结束命中窗口，且房间此刻没有更早开始仍未结束的其他实例。
- 事件 payload 中 `start_at` / `end_at` / `triggered_at` 保持 UTC，另含 `timezone`、`start_local`、
  `end_local`、`triggered_at_local` 便于下游直接展示。
- 投递目标由规则 `config` 决定：`targets` 默认三种全投，`webhook_url` 优先取 `config`，其次房间 `webhook_url`；
  `downlink_kind` 默认 `schedule.event`，`payload` 会浅合并进事件 payload。
- `pubsub` 总是可投递；`downlink` 在房间未绑定设备槽位时标记 `skipped`；`webhook` 在都没有 URL 时 `skipped`。
- Webhook 重试退避 `next_attempt_at = now + min(5 × 2^attempts, 300)` 秒，
  `attempts >= SCHEDULE_WEBHOOK_MAX_ATTEMPTS` 后置为 `failed`。
- 多副本通过 Redis 锁（`schedule:engine:tick` / `schedule:engine:deliver` / `schedule:engine:rebuild`）保证
  同一轮只有一个副本执行；事件与投递状态在 PostgreSQL 中共享。

### 配置

| 环境变量 | 默认 | 说明 |
| --- | --- | --- |
| `SCHEDULE_ENGINE_ENABLED` | `true` | 是否启动课表事件引擎 |
| `SCHEDULE_TICK_SECONDS` | `15` | 触发扫描 / 投递周期，5..=300 |
| `SCHEDULE_HORIZON_DAYS` | `30` | 排课实例物化天数，1..=366；写接口重建也用它 |
| `SCHEDULE_REBUILD_INTERVAL_SECONDS` | `3600` | 全量重建周期，60..=86400 |
| `SCHEDULE_PUBSUB_CHANNEL` | `qe-schedule:events` | 事件 Redis Pub/Sub 频道 |
| `SCHEDULE_DOWNLINK_NOTIFY_CHANNEL` | `qe-device-pki:downlink` | 设备下行通知频道（与 device-pki 一致） |
| `SCHEDULE_WEBHOOK_TIMEOUT_SECONDS` | `5` | 单次 webhook 超时，1..=60 |
| `SCHEDULE_WEBHOOK_MAX_ATTEMPTS` | `5` | 最大投递尝试次数，1..=20 |
| `SCHEDULE_DELIVERY_BATCH` | `50` | 每轮最多处理的到期投递数，1..=500 |

### 总管理节点（未来）

`areas.parent_id`、`rooms.metadata` 与事件 payload 均为将来**总管理节点 / 子管理节点**预留：
多个房间节点可由总管理节点统一下发区域/房间/课表，并汇总事件与投递状态。
本版本不实现节点间同步、学期管理、教师/课程独立实体、调课审批流与事件归档。

## 设备与区域/房间

设备接入把 `device-pki` 管理的设备纳入区域/房间：房间列表直接展示设备注册/在线状态，负责人可以在房间里
**添加、迁移、解绑设备**，并直接从房间生成设备注册 token/链接。
设备注册记录仍由 `device_enrollment` / `device-pki` 管理；**绑定的唯一事实来源是
`rooms.device_slot` ↔ `device_enrollment.device_slot`**（同一 slot 只允许一个非终态证书）。
设计契约见 [`docs/schedule-design.md`](docs/schedule-design.md) §11，权限迁移为 `migrations/0010_device_room.sql`。

### 房间列表里看设备

`Room` 响应（列表 / 详情 / 创建 / 更新）自动带只读 `device` 摘要；房间未绑定槽位时为 `null`：

```json
{
  "device_slot": "node-101",
  "serial": "3f2a9c…",
  "status": "active",
  "online": "online",
  "last_seen_at": "2026-11-02T08:00:00Z",
  "pending_token": false
}
```

- `serial = null`：槽位已分配但设备尚未注册；
- `pending_token = true`：存在未使用且未过期的注册 token，等待设备注册；
- `status` 来自 `device_enrollment.status`，`online` / `last_seen_at` 来自 `device_session`，主 API 只读。

### 添加设备（签发注册 token → 设备注册）

1. 负责人对房间调用 `POST /rooms/{id}/device-token`（房间尚无槽位时自动用房间 code 分配，冲突追加 8 位 id 片段）；
2. 响应返回一次性明文 `token`（库中只存 SHA-256）与注册链接 `link`：

   ```json
   {
     "token": "…64 位 hex…",
     "kind": "auto",
     "device_slot": "node-101",
     "expires_at": "2026-11-02T08:15:00Z",
     "link": "https://<enroll-endpoint>/enroll?token=…&slot=node-101"
   }
   ```

   `kind` 可选 `auto`（自动注册）/ `approval`（需审批），缺省 `auto`；`ttl_secs` 缺省 `900`（60..=86400）；
3. 设备端使用该链接完成注册后，房间 `device` 自动从「等待注册」变为对应注册/在线状态，无需额外操作。

### API 与迁移 / 解绑

| 方法 | 路径（`/api/v1/admin` 下） | 权限 | 说明 |
| --- | --- | --- | --- |
| GET | `/devices?area_id=&room_id=&status=&assigned=&keyword=&page=&page_size=` | `device:read` | 设备分页（含房间/区域与在线状态，按 `issued_at` 倒序） |
| GET | `/devices/{serial}` | `device:read` | 设备详情；不存在 404 |
| GET | `/rooms/{id}/device` | `device:read` | 房间设备摘要；未绑定返回 `null`，房间不存在 404 |
| PUT | `/rooms/{id}/device` | `device:manage` | `{"device_slot": "node-101"}` 绑定/迁移；`{"device_slot": null}` 解绑；房间不存在 404 |
| POST | `/rooms/{id}/device-token` | `device:manage` | 确保房间有槽位后签发注册 token |

- **迁移**：`PUT /rooms/{new}/device` 绑定已属于其它房间的槽位时，仓储层在事务内先清空旧房间再绑定；
  审计动作为 `room.bind_device`；
- **解绑**：`PUT /rooms/{id}/device` 传 `{"device_slot": null}`；设备注册记录不删除，`/devices` 仍能看到，
  但 `room_id = null`（未绑定）；审计动作为 `room.unbind_device`；
- **槽位唯一**：沿用 `rooms_device_slot_key` 部分唯一索引，并发绑定冲突返回 409；
- 签发 token 的审计动作为 `room.issue_device_token`。

### 权限

```
device:read    device:manage
```

- `device:read`：查看设备列表/详情与房间设备摘要；
- `device:manage`：绑定/迁移/解绑设备槽位、签发设备注册 token；
- `admin` 角色自动拥有全部权限；`viewer` / `editor` 默认只有 `device:read`。

### 配置

| 环境变量 | 默认 | 说明 |
| --- | --- | --- |
| `PUBLIC_ENROLL_ENDPOINT` | `https://localhost:4440` | 设备注册服务对外地址，用于拼装 token 响应里的 `link`（自动去掉末尾 `/`） |

## 资源与镜像管理（P0）

房间节点运行 Docker 容器、LXC 容器与 KVM/VM，三者用同一套抽象管理产物：
**内容寻址分片 + manifest + Ed25519 签名 + 验证报告**。数据面是 S3 兼容对象存储
（开发 MinIO，目标 RustFS，生产可换 Ceph/云 OSS），控制面在 `qe-api`。
**P0 不实现 OCI Registry 协议**（需要标准 `docker pull` 时见设计文档 §9）。
设计契约见 [`docs/storage-design.md`](docs/storage-design.md) §14，迁移为 `migrations/0011_storage.sql`。

```text
<bucket>/
  objects/{sha256}                   # 分片 / 单文件（内容寻址，天然去重）
  manifests/{manifest_sha256}.json   # 清单（权威标识 = manifest 的 sha256）
  signatures/{manifest_sha256}.sig   # Ed25519 签名，覆盖 manifest 原始字节
```

### S3 / RustFS 配置

| 环境变量 | 默认 | 说明 |
| --- | --- | --- |
| `S3_ENDPOINT` | 无 | S3 兼容端点；与 `S3_BUCKET` **同时**设置才启用本模块 |
| `S3_BUCKET` | 无 | 桶名 |
| `S3_REGION` | `us-east-1` | SigV4 签名区域 |
| `S3_ACCESS_KEY` / `S3_SECRET_KEY` | 空 | 凭证（生产建议走 Secret） |
| `S3_PATH_STYLE` | `true` | MinIO / RustFS 通常需要 path-style |
| `STORAGE_PRESIGN_TTL_SECONDS` | `900` | 预签名 TTL，60..=86400 |
| `STORAGE_SIGNING_KEY_BASE64` | 空 | 32 字节 Ed25519 seed 的 base64；不设则启动时随机生成（**仅开发**） |

未配置 `S3_ENDPOINT` + `S3_BUCKET`（或 `StorageService` 初始化失败）时 API 仍可启动，
所有存储/镜像接口返回 `503 service_unavailable`，其它模块不受影响。

本地起 RustFS（compose profile `storage`）：

```bash
docker compose --profile storage up -d rustfs   # 9000 = S3，9001 = 控制台
# .env
S3_ENDPOINT=http://127.0.0.1:9000
S3_BUCKET=qestudio
S3_ACCESS_KEY=rustfsadmin
S3_SECRET_KEY=rustfsadmin
# 开发可以把 Ed25519 种子固定下来，重启后已签 manifest 仍可验签：
# STORAGE_SIGNING_KEY_BASE64=$(head -c 32 /dev/urandom | base64)
```

进程内集成测试用 fake S3（axum + 内存 map）覆盖上传/验证/发布链路，不依赖真实 RustFS。

### API（`/api/v1/admin`）

| 方法 | 路径 | 权限 | 说明 |
| --- | --- | --- | --- |
| POST | `/storage/objects/presign` | `storage:manage` | `{sha256,size,content_type?}` → `{bucket,object_key,upload_url,expires_at}`，对象键固定 `objects/{sha256}` |
| POST | `/storage/objects` | `storage:manage` | 登记对象：服务端从 S3 读回内容重算 sha256/size，不一致 400 |
| GET | `/storage/objects/{sha256}` | `storage:read` | 对象详情 + 预签名下载地址；未登记 404 |
| POST | `/images` | `image:manage` | 创建制品（`uploaded`）；`name+version+platform` 唯一 |
| GET | `/images?kind=&status=&name=&page=&page_size=` | `image:read` | 镜像分页（`name` 模糊） |
| GET | `/images/{id}` | `image:read` | 详情 + chunks + 最近验证报告 + 已发布通道 |
| POST | `/images/{id}/manifest` | `image:manage` | 提交分片清单 → 写 manifest/签名 → 状态 `manifest_ready` |
| POST | `/images/{id}/verify` | `image:manage` | 同步验证并落报告；通过置 `verified` |
| POST | `/images/{id}/publish` | `image:manage` | `{channel:"dev"\|"test"\|"prod"}`，仅 `verified` 可发布 |
| GET | `/images/{id}/manifest` | `image:read` | manifest JSON + 预签名下载地址 |
| PUT | `/rooms/{id}/images/{image_id}` | `image:manage` | `{channel?}`（缺省 `prod`）分配并创建 `download` transfer |
| DELETE | `/rooms/{id}/images/{image_id}` | `image:manage` | 取消分配（不删镜像） |
| GET | `/transfers?room_id=&page=&page_size=` | `storage:read` | 分发/收集任务（`created_at` 倒序） |
| POST | `/transfers/{id}/retry` | `storage:manage` | 仅 `failed` / `expired` 可重试（重置进度与错误） |
| GET | `/collections?room_id=&page=&page_size=` | `storage:read` | 收集登记列表 |

典型流程（`curl` 片段，token 省略）：

```bash
# 1. 预签名上传一个分片（sha256/size 由客户端先算好）
curl -s -X POST localhost:8080/api/v1/admin/storage/objects/presign \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"sha256":"<64位hex>","size":1048576,"content_type":"application/octet-stream"}'
# 2. PUT 到 upload_url，然后登记对象
curl -s -X POST localhost:8080/api/v1/admin/storage/objects \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"sha256":"<64位hex>","size":1048576}'
# 3. 创建镜像 → 提交 manifest → 验证 → 发布
curl -s -X POST localhost:8080/api/v1/admin/images \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"web-console","version":"2026.10.31-1","kind":"vm_disk","format":"qcow2","platform":"linux/amd64"}'
# ...
```

### Manifest 结构

`POST /images/{id}/manifest` 的请求体：

```json
{
  "total_size": 16777216,
  "parts": [
    { "index": 0, "sha256": "ab12…", "size": 16777216, "offset": 0 }
  ],
  "runtime": {
    "docker": { "image_ref": "web-console:2026.10.31-1" },
    "lxc": { "distro": "ubuntu", "release": "24.04" },
    "vm": { "os": "windows-11", "disk_size": 34359738368 }
  },
  "annotations": {}
}
```

服务端据此生成的 manifest（存 `manifests/{digest}.json`，字段顺序固定，签名与 digest 都基于这份字节）：

```json
{
  "schema": 1,
  "kind": "vm_disk",
  "name": "web-console",
  "version": "2026.10.31-1",
  "platform": "linux/amd64",
  "format": "qcow2",
  "compression": "none",
  "total_size": 16777216,
  "chunks": [
    { "index": 0, "object_key": "objects/ab12…", "sha256": "ab12…", "size": 16777216, "offset": 0 }
  ],
  "runtime": { "docker_image_ref": null, "lxc_distro": null, "lxc_release": null, "vm_os": "windows-11", "vm_disk_size": 34359738368 },
  "annotations": {},
  "created_at": "2026-10-31T05:00:00Z",
  "created_by": "<uuid>"
}
```

- `version` 只用于展示/排序；**权威标识是 manifest 的 sha256**（即 `manifest_digest`）。
- 分片 `index`/`offset` 必须从 0 连续、`size > 0`、sha256 不重复，`total_size` 必须等于分片大小之和；
  每个分片必须先以 `objects/{sha256}` 登记且 size 一致，否则 400。
- P0 只接受 `compression = none`；`zstd` / `gzip` 字段保留，传非 `none` 直接 400。
- 签名单独存 `signatures/{digest}.sig`（64 字节 Ed25519），manifest JSON 里不含 `signature` 字段。

### 验证清单（`POST /images/{id}/verify`）

| 检查 | 说明 |
| --- | --- |
| `chunk.{i}.sha256` | 从 S3 读回分片并重算 sha256，与 manifest 一致 |
| `chunk.{i}.size` | 读回字节数与该分片 `size` 一致 |
| `total_size` | 所有分片实际字节数之和等于 `total_size` |
| `magic` | qcow2 `QFI\xfb`；tar / docker-archive / oci-layout 在 offset 257 有 `ustar`；raw 跳过 |
| `manifest.digest` | `sha256(manifest 原始字节) == manifest_digest` |
| `manifest.signature` | 用 `signatures/{digest}.sig` 做 Ed25519 验签 |

- 报告结构：`{"manifest_digest":"…","checks":[{"name":"chunk.0.sha256","ok":true,"detail":"…"}],"errors":[]}`，
  写入 `image_verifications`（保留历史）。
- 全部通过 → `status = verified`；任何一项失败 → `report.ok = false`，状态回到 `manifest_ready`，
  修好分片后可重新提交 manifest/再次验证（**fail-closed**）。
- 已发布/废弃/吊销的镜像不能重新验证（409），避免把 `published` 降级。

### 权限与审计

```
storage:read    storage:manage
image:read      image:manage
```

- `storage:read` / `image:read`：查看对象、任务、镜像与验证报告；
- `storage:manage` / `image:manage`：上传登记、提交 manifest、验证、发布、分配/取消分配、重试任务；
- `admin` 角色自动拥有全部权限；`viewer` / `editor` 默认只有 `storage:read` 与 `image:read`。
- 审计动作：`storage.object.register`、`image.create` / `verify` / `publish` / `assign` / `unassign`、`transfer.retry`。

### P1 说明

- 节点侧 agent（独立 Rust 二进制）：拉 manifest → 验签 → 逐 chunk 校验 → 按 kind 导入
  （`docker load` / `ctr images import`、LXC/Incus rootfs、`qemu-img` + libvirt），上报 `transfer.completed`；
  与课表事件联动（上课前分发）。
- zstd/gzip 压缩与解压校验、chunk 复用/增量更新、S3 Event 通知兜底、收集上传（`collections`）、
  配额/保留期/GC、`transfers`/`collections` 的状态过滤、只读 Registry（Zot facade）、Trivy 安全扫描。

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
| `S3_ENDPOINT` | 空 | 对象存储端点；与 `S3_BUCKET` 同时设置才启用资源/镜像模块 |
| `S3_BUCKET` | 空 | 对象存储桶名 |
| `S3_REGION` | `us-east-1` | SigV4 区域 |
| `S3_ACCESS_KEY` / `S3_SECRET_KEY` | 空 | 对象存储凭证 |
| `S3_PATH_STYLE` | `true` | MinIO/RustFS 用 path-style |
| `STORAGE_PRESIGN_TTL_SECONDS` | `900` | 预签名有效期，60..=86400 |
| `STORAGE_SIGNING_KEY_BASE64` | 空 | Ed25519 seed（32 字节 base64），不设则每次启动随机生成 |
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
