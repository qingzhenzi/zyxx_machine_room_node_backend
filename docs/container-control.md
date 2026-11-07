# 容器生命周期 API（Incus 后端）

> 状态：**MVP 已完成并实测**（2026-11-07 晚）。课程环境容器由镜像管道导入 Incus；
> 本 API 提供容器生命周期与快照/恢复，补齐 VM 之外的另一半实验环境管理。
> 范围：Incus/LXC 容器；Docker 容器暂不在此 API（`qe-container` 有 Docker 引擎，未接控制面）。

## 链路

```
控制台/API
  POST /api/v1/admin/machines/{serial}/container/commands   （权限 vm:control）
    → machine_command_results(kind=container.command, pending)
    → device_downlink kind=container.command
    → agent dispatcher（node/both 角色）→ qe-agent-node/src/container_control.rs
    → incus CLI（QE_INCUS_BIN 可覆盖，默认 incus）
    → POST /machine/command/result（mTLS）→ 结果表
  GET  .../container/commands/{request_id} 轮询终态（复用 VM 结果查询）
```

## 请求/响应

```json
// POST body（阶段 1 的 {op,name} 兼容）
{"op":"start","args":{"name":"debian12-cfg"}}
// 202 {"request_id":"..."}
// GET .../container/commands/<id>
{"request_id":"...","status":"ok|error|timeout","data":{"name":"debian12-cfg","result":"ok"}}
```

## op 白名单（9 个）

| op | args | 说明 |
|---|---|---|
| `list` | 无 | `incus list --format json`，data `{containers:[{name,status,type,ipv4}]}` |
| `start` / `stop` / `restart` | `{name}` | 生命周期；`stop` 可带 `force=true`（`incus stop --force`，容器内 init 不响应优雅停机时用） |
| `delete` | `{name, force?}` | 删除容器；`force=true` 删除运行中容器 |
| `snapshot.create` | `{name, snapshot}` | 创建 Incus 快照 |
| `snapshot.list` | `{name}` | data `{snapshots:[{name,created_at,stateful}]}` |
| `snapshot.restore` / `snapshot.delete` | `{name, snapshot}` | 恢复/删除快照 |

## 校验与限制

- `name`/`snapshot`：`[A-Za-z0-9._-]{1,64}`（防 argv 注入；参数逐个传 argv，不经 shell）；
- `list` 不接受参数；`snapshot.*`（除 list）必须带 snapshot；未知 args 字段 400；
- 权限复用 `vm:control`（实验环境控制）；设备不存在 404、无存活证书 503；
- pending 超时 120s（`incus` 命令自身 120s 超时；`stop` 若容器 init 不响应优雅信号，
  建议显式 `force=true`，否则可能等到超时）；
- 容器 GUI 不在此 API：Incus 容器默认 headless；需要桌面需容器内 XxVNC，不在本期；
- 真机验证要求主机安装 Incus（7.5.1 本机验证）且 agent 用户在 incus-admin 组。

## 实测（2026-11-07，Lead）

- 本机导入可启动 Debian rootfs 作为 Incus 镜像 `qe-debian-e2e`（27MB），`incus init → API`：
  - `list` ✅ 返回容器与状态/IP
  - `start` ✅ 容器 RUNNING
  - `snapshot.create/list/restore/delete` ✅（快照名、创建时间、stateful=false）
  - `stop {force:true}` ✅
  - `delete {force:true}` ✅ 容器消失
- 日志：`tmp/logs/container-live.log`；后端 341 lib、agent qe-agent-node 145 lib 全绿。
