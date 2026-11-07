# 总控阶段 2：多控制节点联邦（MVP）

> 状态：**MVP 已实现**（2026-11-07）。只读聚合：节点注册 / 手动刷新 / 跨节点总览；
> 不做跨节点下发、统一登录与定时自动同步（列入后续阶段）。
>
> 设计背景见 [`total-control.md`](total-control.md) §3；本文件是实现与运维说明。

## 1. 范围

| 能力 | MVP 状态 | 说明 |
| --- | --- | --- |
| 节点注册（name/base_url/token） | ✅ | 存 `federation_nodes`，token 用于拉取远端摘要 |
| 远端摘要拉取（手动刷新） | ✅ | `GET <远端>/api/v1/admin/federation/summary`，5s 超时 |
| 跨节点总览（聚合 KPI） | ✅ | 本节点 + 所有 enabled 节点；`/federation` 前端页 |
| 在线/离线判定 | ✅ | 最近刷新成功且 ≤5min → online；否则 offline/unknown |
| 审计 | ✅ | `federation.node.create/delete/refresh` |
| 定时自动刷新 | ❌ | MVP 手动刷新（后续加 worker） |
| 跨节点下发/统一登录 | ❌ | 阶段 2b/2c |

架构（MVP 为「各控制节点互相拉取只读摘要」，不引入独立 all_room_node 服务）：

```text
 浏览器 /federation
   └─ qe-api A（主节点，8080）
        ├─ GET /admin/federation/overview  ← 本地 summary + federation_nodes 缓存
        └─ POST /admin/federation/nodes/{id}/refresh
             └─ HTTP GET http://<节点 B>/api/v1/admin/federation/summary
                  header: x-federation-token: <B 的 FEDERATION_TOKEN>
```

## 2. 数据模型（迁移 `0034_federation_nodes.sql`）

```sql
federation_nodes(
  id uuid pk, name text unique, base_url text, token text,
  enabled bool default true, last_seen_at timestamptz,
  last_status text,          -- online / offline / NULL
  cached_summary jsonb,      -- 最近一次成功的远端 summary 响应
  created_at, updated_at
)
```

- `cached_summary` 保存整份远端响应 `{node_name, generated_at, counts}`；
- 刷新失败**不清空**缓存（页面可展示上次成功数据 + offline 状态）；
- token 为 MVP 明文存储，生产应迁移到密文/外部 Secret（见 §7）。

## 3. API

| 方法/路径 | 鉴权 | 说明 |
| --- | --- | --- |
| `GET /api/v1/admin/federation/summary` | `x-federation-token`（env `FEDERATION_TOKEN`）**或** admin JWT | 本节点只读计数摘要，供其他节点拉取 |
| `GET /api/v1/admin/federation/nodes` | admin JWT + `device:read` | 节点列表（**不回显 token**） |
| `POST /api/v1/admin/federation/nodes` | admin JWT + `device:read` | `{name, base_url, token}`；名称唯一（409） |
| `DELETE /api/v1/admin/federation/nodes/{id}` | admin JWT + `device:read` | 删除本地联邦记录（不影响远端节点） |
| `POST /api/v1/admin/federation/nodes/{id}/refresh` | admin JWT + `device:read` | 拉远端 summary；成功 online、失败 offline + 502 |
| `GET /api/v1/admin/federation/overview` | admin JWT + `device:read` | 本节点 + enabled 节点聚合 |

`overview` 响应要点：

```jsonc
{
  "node_name": "control-node",
  "generated_at": "...",
  "local": { "rooms": 18, "areas": 13, "machines": 17, "online_machines": 0, "open_tickets": 6, "images": 9 },
  "nodes": [ { "id": "...", "name": "...", "base_url": "...", "status": "online|offline|unknown",
               "last_seen_at": "...", "summary": { ... } } ],
  "totals": { // 含本节点；只有 online 节点且能解析摘要的计数计入
    "nodes_total": 2, "nodes_online": 2, "nodes_offline": 0,
    "rooms": 18, "areas": 13, "machines": 17, "online_machines": 0, "open_tickets": 6, "images": 9
  }
}
```

口径：
- `summary` 计数来自各节点 `device/room/ticket/image` 表的现有数据（`rooms/areas` 只算 active，
  `machines` 算 issued/active 设备，`online_machines` = device_session online 或 5min 内快照，
  `open_tickets` 排除 resolved/closed/cancelled）；
- `nodes_total` = 本节点 + enabled 远端节点数；本节点始终在线；
- `status=unknown` 表示从未成功刷新过；`offline` 表示最近一次刷新失败或成功时间超过 5min；
- 列表按名称排序，token 永不返回。

## 4. 环境变量

| 变量 | 端 | 说明 |
| --- | --- | --- |
| `FEDERATION_TOKEN` | 每个控制节点 | 其他节点用 `x-federation-token` 拉取本节点 summary；**未配置 = 只接受 admin JWT** |
| `BIND_ADDR` | qe-api | 从实例需要覆盖（如 `127.0.0.1:8081`），不要改 `.env` 里的主实例端口 |

前端权限：页面「总控联邦」（`/federation`）**仅 admin 角色可见/可进**；API 仍按 `device:read` 授权
（便于以后给节点管理员只读总控页）。

## 5. 本机启动两个实例（实测命令）

> 约定：8080 为主实例（正常 `.env`），8081 为从实例（独立 DB `qestudio2`）。
> `dotenvy` **不会覆盖已存在的环境变量**，所以下面用内联 env 覆盖 `BIND_ADDR`/`DATABASE_URL`，
> 不需要（也不要）改 `.env` 里的主实例端口。

```bash
cd room_node

# 1) 从实例独立库（迁移由 qe-api 启动自动应用；也可用 psql 手工建库）
PGPASSWORD=qestudio psql -h 127.0.0.1 -U qestudio -d postgres \
  -c "CREATE DATABASE qestudio2;"   # 已存在会报错，可忽略

# 2) 主实例：正常启动（8080，使用 .env）
cargo run -p qe-api

# 3) 从实例：另开终端（8081 + qestudio2；关掉后台 worker 避免两实例争抢）
set -a; source .env; set +a
DATABASE_URL="postgres://qestudio:qestudio@127.0.0.1:5432/qestudio2" \
BIND_ADDR="127.0.0.1:8081" \
FEDERATION_TOKEN="node-b-dev-token" \
DEVICE_PRESENCE_ENABLED=false SCHEDULE_ENGINE_ENABLED=false \
TICKETS_AUTO_ENABLED=false TICKET_EMAIL_ENABLED=false \
RUN_MIGRATIONS=true \
cargo run -p qe-api

# 主实例同样配置 FEDERATION_TOKEN（供 B 反向拉取；只读 MVP 里可选）
```

注册/刷新（对主实例 8080 操作）：

```bash
TOKEN=$(curl -s -X POST http://127.0.0.1:8080/api/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{"username":"admin","password":"Admin@12345"}' | jq -r .access_token)

# 注册从实例（token = 从实例启动时的 FEDERATION_TOKEN）
curl -s -X POST http://127.0.0.1:8080/api/v1/admin/federation/nodes \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"name":"机房 B","base_url":"http://127.0.0.1:8081","token":"node-b-dev-token"}'

# 刷新 → online；再 GET overview 看聚合
curl -s -X POST http://127.0.0.1:8080/api/v1/admin/federation/nodes/<id>/refresh \
  -H "authorization: Bearer $TOKEN"
curl -s http://127.0.0.1:8080/api/v1/admin/federation/overview \
  -H "authorization: Bearer $TOKEN"
```

浏览器打开 `http://localhost:5173/federation`（admin 登录）：顶部聚合 KPI、本节点摘要、
节点列表（名称/地址/状态/最近刷新/摘要）、添加/刷新/删除。

## 6. 实测记录（2026-11-07）

环境：本机 Postgres/Redis 已起；**8080 被现有主实例占用**，因此用同样的代码在
`18080`（DB `qestudio`）/ `18081`（DB `qestudio2`）跑双实例验证，流程与 8080/8081 完全一致。

| 步骤 | 结果 |
| --- | --- |
| 迁移 | `qestudio` 33→**34/34**；`qestudio2` 全新建库并迁移 **34/34** |
| 远端摘要 | `GET :18081/admin/federation/summary` + `x-federation-token` → 200，counts 全 0（新库） |
| 注册节点 | `POST /federation/nodes` → 201，`status=unknown` |
| 刷新 B（在线） | 200，`status=online`、`last_seen_at` 更新、`cached_summary` 写入 |
| overview（B 在线） | `nodes_total=2, nodes_online=2, nodes_offline=0`；本地 counts 正确（rooms 18/areas 13/machines 17/open_tickets 6/images 9） |
| 停 B 后刷新 | **502**（`bad_gateway`，远端连接失败），节点标记 `offline`，保留上次缓存 |
| overview（B 离线） | 节点 `status=offline`；`nodes_online=1, nodes_offline=1`，KPI 只计在线节点 |
| 删除节点 | `DELETE` → 204；`federation_nodes` 清空 |
| 清理 | 两个临时实例已停止；`qestudio2` 保留（可直接作为 8081 从实例库） |

结论：注册 → 刷新（online）→ 聚合 → 停远端 → 刷新（offline/502）→ 删除 的闭环通过。

## 7. 遗留 / 下一步

1. **定时刷新**：MVP 只有手动刷新；后续加后台 worker（如 60s 一轮，带并发上限/退避）。
2. **跨节点下发**：阶段 2b 走内部 mTLS API，复用 device-pki 证书体系，不在本 MVP 范围。
3. **统一登录/权限**：阶段 2c 增加 `fleet:read/fleet:manage` 与节点级数据隔离。
4. **token 安全**：当前明文存库；生产改密文（如 AES-GCM + KMS）或外部 Secret，并强制 HTTPS。
5. **HTTPS/反代**：MVP 用 http；生产 base_url 应为 https，并由反代限制 `/federation/summary` 来源 IP。
6. **多副本**：前端页面与 API 可多副本；刷新动作只写本节点 DB，天然幂等（无分布式锁）。
7. **限流/SSRF**：目前仅 admin 可注册节点（可填任意 URL）；生产建议域名白名单/出网策略。
