# 总控（all_room_node）设计

> 状态：阶段 1 已有雏形（单控制节点内的多区域/多房间总览）；阶段 2（多控制节点联邦）为设计稿，未实现。

## 1. 概念澄清

现有代码里有两个容易混淆的「总控」：

| 名称 | 含义 | 现状 |
|------|------|------|
| 控制节点（room_node） | 一个机房/楼层的 Web+API+DB+device-pki，管理本节点下的房间/设备/课表/工单 | **已实现** |
| 总控（all_room_node） | 管理**多个**控制节点的联邦层：跨机房总览、跨节点排课/镜像分发/告警汇总 | **未实现**（目录在旧文档里是空的） |

本文件只讨论后者。

## 2. 阶段 1（已具备）：单控制节点内的「总览」

现有控制台已提供：
- `/areas` 区域总览、`/areas/{id}` 区域详情、`/rooms` 房间总览、`/machines` 机器总览；
- `/schedule-board` 跨全部房间的排课工作台；
- `/tickets/insights` 跨房间工单分析；
- `/devices`、`/device-events`、`/tickets` 的全局列表与 SSE。

所以单机房场景下，「总控视图」不需要新服务，直接复用控制台。
**验收**：一个控制节点内 ≥2 个区域、≥10 个房间能在总览/排课/工单页正常聚合（现网数据已验证）。

## 3. 阶段 2（设计稿）：多控制节点联邦

### 3.1 目标场景

一所学校多个机房各自部署一个控制节点（room_node）；教务处/信息中心需要：
- 跨机房统一总览：设备在线率、健康分、未处理工单、课表占用；
- 跨机房下发：镜像分发任务、巡检计划、WOL；
- 单点登录 + 统一权限（总控看到所有节点，节点管理员只看本节点）。

### 3.2 架构

```
all_room_node（新服务，独立部署）
  ├── 节点注册表：node_id / base_url / 公钥 / 状态 / 版本 / 能力
  ├── 只读同步：从各 control node 拉取聚合视图（增量/定时）
  ├── 命令下发：通过各节点的 mTLS 内部 API 触发（镜像/巡检/WOL）
  └── 前端：跨节点总览 + 深链接跳转到各节点控制台
```

关键设计：
1. **节点即租户**：所有聚合表带 `node_id`，权限模型新增 `fleet:read` / `fleet:manage`；
   节点管理员在总控上只映射到自己的 `node_id`。
2. **控制节点零改造优先**：阶段 2a 只读同步走现有 admin API（只读账号 + API Token），
   不改控制节点；阶段 2b 再加内部 mTLS API（`/internal/fleet/*`）做下发。
3. **时钟与去重**：拉取用 `updated_at` 水位；幂等 upsert；节点离线标记而非删除。
4. **跨节点镜像分发**：总控只做「任务编排」，实际分发仍由各控制节点的 device-pki 完成；
   跨机房带宽问题留给 P2P 联邦（阶段 3，与 qep2p 的 relay 结合）。

### 3.3 数据模型（草案）

```sql
fleet_nodes(id uuid pk, name text, base_url text, public_key text,
            status text, last_seen_at timestamptz, version text);
fleet_room_snapshots(node_id uuid, area_id text, room_id text, name text,
                     device_online int, health_score int, updated_at timestamptz,
                     PRIMARY KEY(node_id, area_id, room_id));
fleet_ticket_summary(node_id uuid, open_count int, overdue_count int, updated_at timestamptz);
fleet_tasks(id uuid, node_id uuid, kind text, payload jsonb, status text, ...);
```

### 3.4 阶段拆解与验收

| 阶段 | 内容 | 估时 | 验收 |
|------|------|------|------|
| 2a | 节点注册 + 只读同步（设备/房间/工单/课表）+ 跨节点总览页 | 5–8d | 两个独立 compose 控制节点在总控同时可见，数据 5min 内同步 |
| 2b | 内部 mTLS API + 跨节点下发（WOL/巡检/镜像任务）+ 任务状态 | 5–8d | 总控下发 WOL，目标节点执行并回传结果 |
| 2c | 统一登录/权限（fleet:read/manage）+ 审计 | 3–5d | 节点管理员只见自己节点；总控账号跨节点只读 |
| 3 | 跨机房 P2P（qep2p relay 联邦） | 5–10d | 见 P2P 多 seeder 实验 |

## 4. 与当前代码的关系

- **不修改**现有 room_node 的 single-node 语义；总控是叠加服务。
- `all_room_node/` 目录目前为空，阶段 2a 可直接在其中新建 crate/服务。
- 与「教师端」的关系：教师端是控制台的一个角色视图（见 docs/teacher-console.md），
  总控是跨控制节点的联邦，两者独立。
