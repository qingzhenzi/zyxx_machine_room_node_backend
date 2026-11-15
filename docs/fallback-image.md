# fallback 镜像（设备本地缓存）

> 状态：**服务端已实现（2026-11-15）；agent 侧缓存/兜底接入待实现**。
>
> 关联契约：[`startup-provisioning.md`](startup-provisioning.md) §2 / §3 / §4.1。
> 本文件说明「fallback 镜像」的配置语义、下发形状、设备本地缓存职责与禁用方式。

## 1. 要解决的问题

`qe-agent` 连不上控制面（网络故障、控制节点维护/宕机）时，`qe-local-fallback`
watchdog 会尝试启动本机的一台「兜底 VM」；但如果这台机器的镜像从未成功拉取过，
兜底也会失败。fallback 镜像提供的是：

**服务端声明「这个房间用哪个镜像兜底、是否启用」→ agent 提前把它缓存到设备本地
→ 控制面不可达时用本地缓存启动兜底环境。**

服务端只做「声明 + 校验 + 下发」；**镜像缓存在设备侧**，由 agent 负责下载、校验、
持久化与离线使用（server 无法观测设备磁盘，也不远程删除本地缓存）。

## 2. 配置语义

fallback 配置属于房间启动配置（`room_startup_profiles.config` JSONB），与
`auto_pull` / `auto_start_vm` / `images` / `vms` 同级：

```json
{
  "auto_pull": true,
  "auto_start_vm": true,
  "images": [{ "image_id": "204cb75d-ac1b-4492-b8b3-808d2270070b" }],
  "vms": [],
  "fallback_enabled": true,
  "fallback_image_id": "9f1d0f9a-7c2e-4a51-9d1c-3b0f7a2a1c77"
}
```

| 字段 | 类型 | 缺省 | 说明 |
|---|---|---|---|
| `fallback_enabled` | bool | `false` | 是否启用 fallback 镜像（关闭时 `/agent/startup` 不再下发 fallback 条目）。 |
| `fallback_image_id` | UUID \| null | `null` | fallback 镜像 id；`fallback_enabled=true` 时**必填**（400 `config.fallback_image_id ...`）。 |

规则：

- **与正常镜像同等级**：镜像必须存在、`status=published`、`manifest_digest` 非空；
  保存启动配置时会自动创建 `image_assignments(channel='prod')` 与
  `transfers(direction='download', status='pending')`，否则 agent 在
  `POST /resources/presign` 会因「镜像未分配到该房间」被 403。
- **单独标记/单独下发**：不占用 `images[]` 的 0..=8 名额；`/agent/startup` 用独立的
  `fallback_enabled` + `fallback_image` 字段下发（条目形状与 `images[]` 完全相同，
  agent 可直接复用 `image.pull` 链路）。
- **可配置关闭**：`fallback_enabled=false` 时服务端不产生新的 transfer/assignment，
  也不在下发响应中携带镜像条目；已选 id 可以保留，便于再次启用。
- **旧配置兼容**：两个字段都有 serde 缺省值，0035 时代的旧 JSONB 直接反序列化为
  「未启用」，无需迁移；读一次再写回即完成形状升级。
- 可以与 `images[]` 指向同一个镜像（id 相同）：服务端幂等只补一份 transfer/assignment；
  agent 侧按 `image_id` 去重即可。
- fallback 依赖房间启动配置存在且 `enabled=true`：只配置了「房间默认底包」
  （`/admin/rooms/{id}/image-default`）而没有启动配置时，不会触发 fallback 下发。

### 存储位置

| 层 | 位置 | 说明 |
|---|---|---|
| 服务端 | `room_startup_profiles.config` JSONB | 无新表/列，随启动配置一起 `revision+1`、审计、下发 `startup.update`。 |
| 服务端 | `image_assignments` / `transfers` | 启用时幂等补齐（与 `images[]` 同一套逻辑）。 |
| 设备 | agent 数据目录（建议 `<DEVICE_DATA_DIR>/fallback/`） | 缓存本体 + digest；由 agent 实现，服务端不感知。 |

## 3. 管理端 API

`GET` / `PUT /api/v1/admin/rooms/{room_id}/startup`（权限 `vm:control`）：

- 请求 `config` 增加 `fallback_enabled` / `fallback_image_id`（PUT 校验见 §2）；
- GET / PUT 响应的 `config` 原样回显两个字段；
- 响应的 `images[]` 之外新增 `fallback_image`（`null` 或与 `images[]` 同形的条目，
  含 `assigned` / `transfer_id` / `manifest_digest` 等），便于管理台展示与排障。
- 管理台（`frontend/src/features/rooms/RoomStartupPanel.vue`）提供「启用 fallback 镜像」
  开关 + 已发布 `vm_disk` 镜像下拉。

## 4. 设备 mTLS 下发（`GET /agent/startup`）

启用时在原有响应上追加：

```json
{
  "revision": 3,
  "enabled": true,
  "auto_pull": true,
  "auto_start_vm": true,
  "images": [ /* 原有正常镜像 */ ],
  "vms": [ /* 原有 VM 声明 */ ],
  "fallback_enabled": true,
  "fallback_image": {
    "image_id": "9f1d0f9a-7c2e-4a51-9d1c-3b0f7a2a1c77",
    "transfer_id": "a105a24f-cc3e-48dc-ad37-2c3368a36829",
    "name": "win10-fallback",
    "version": "1.0.0",
    "kind": "vm_disk",
    "format": "qcow2",
    "platform": "windows/amd64",
    "manifest_digest": "sha256:...",
    "total_size": 5960000000,
    "chunk_count": 91,
    "presign_endpoint": "/resources/presign"
  }
}
```

- **关闭时**：`fallback_enabled=false`、`fallback_image=null`（不下发条目）。
- 无配置行 / 配置行 `enabled=false` / 槽位未绑定房间：沿用原语义（200 +
  `revision=0`、`enabled=false`、空列表 + fallback 关闭）。
- 启用了 fallback 但镜像已下架/被删除：`fallback_enabled=true`、`fallback_image=null`，
  agent 应告警并保留旧缓存（不要当成本地缓存损坏）。
- 字段全部 snake_case；旧 agent 的 serde 未开 `deny_unknown_fields`，会忽略新字段，**向后兼容**。

## 5. agent 侧接入点（room_agent，待实现）

1. `crates/qe-device-pki-client/src/startup.rs` 的 `StartupReply` 增加
   `#[serde(default)] fallback_enabled: bool` 与 `#[serde(default)] fallback_image: Option<StartupImage>`。
2. `crates/qe-agent-node/src/startup.rs` reconcile：
   - `fallback_enabled && fallback_image.is_some()` 且 `auto_pull=true` 时，按普通镜像
     复用 `ImagePullJob`（P2P/HTTP → 逐片校验 → import）拉取；已存在则跳过；
   - 缓存到 **持久目录**（建议 `<DEVICE_DATA_DIR>/fallback/`，不要用会被清理的 staging），
     并在 `startup-state.json` 记录 `fallback_image_id` / `manifest_digest` / 本地卷名；
   - 下载/校验失败只 warn，下轮重试；不得删除其它镜像或 VM；
   - `fallback_enabled=false` 时不下载、不更新本地 fallback 记录（已有缓存可保留）。
3. `qe-local-fallback` watchdog（`room_agent/deploy/qe-local-fallback/`）在控制面超时后
   优先使用该缓存镜像对应的 libvirt 卷启动兜底 VM（当前实现用
   `QE_LOCAL_FALLBACK_VM` 指定的本机 VM；接入时把"镜像 → 卷名推导/启动"串起来）。
4. 日志统一带 `startup` 关键字，便于 `journalctl -u qe-agent | grep startup` 排障。

## 6. 如何禁用

| 方式 | 操作 | 效果 |
|---|---|---|
| 推荐：房间级关闭 | 管理台「启动配置 → 启用 fallback 镜像」关掉并保存（PUT `fallback_enabled=false`） | `/agent/startup` 不再下发 fallback；agent 下一轮 reconcile（≤60s 或收到 `startup.update`）停止缓存；**本地已有缓存不会被远程删除** |
| 整个启动配置停用 | PUT `{ "enabled": false, ... }` | 该房间不再自动拉镜像/建 VM，也不下发 fallback |
| 设备级完全关闭 | agent 环境变量 `QE_STARTUP_ENABLED=0` 并重启 agent | 不再拉启动配置（回退为纯人工/下行控制） |
| 运维清理 | 手动删除设备 `<DEVICE_DATA_DIR>/fallback/` 与对应 libvirt 卷 | 释放磁盘；删除前确认不影响兜底/正常 VM |

服务端没有「远程删除设备缓存」的 API（安全边界：控制面不应能任意删除设备文件）。

## 7. 安全边界与已知限制

- 只读底包 + qcow2 差分盘语义不变；agent 绝不 delete/shutdown 配置之外的 VM。
- 服务端只下发 `published` 且 manifest 完整的镜像；manifest 逐片校验由 agent 拉取链路负责。
- 服务端**无法验证设备本地缓存完整性**：agent 必须在启动兜底前按 `manifest_digest`
  校验缓存（复用 `image.pull` 的校验逻辑），否则离线启动可能使用损坏/不完整缓存。
- fallback 只覆盖「镜像层」：VM 规格/网络配置仍来自启动配置的 `vms[]`（离线时是
  agent 上一次成功拉取并记录的配置），不保证离线时配置一定是最新。
- Windows 等大镜像（数 GB～10GB）会长期占用设备磁盘，需要预留空间并在管理端/文档提示。
- 镜像下架/删除后服务端不再下发 fallback 条目，但**设备上的旧缓存仍可用**，需要运维清理；
  这属于"离线可用"与"安全撤回"之间的已知取舍。
- 并发：多副本 qe-api 同时 PUT 时 `revision` 由 SQL 原子自增；agent 以 revision 最大者为准。

## 8. 服务端实现位置

| 位置 | 内容 |
|---|---|
| `crates/db/src/startup.rs` | `StartupConfig.fallback_enabled` / `fallback_image_id`（serde 缺省，向后兼容）；upsert 时幂等补 fallback 的 transfer/assignment。 |
| `crates/api/src/api/admin/startup.rs` | PUT 校验（启用必须有 id、published + manifest）与 GET/PUT 响应 `fallback_image` 回显/审计。 |
| `crates/device-pki/src/startup.rs` | `/agent/startup` 下发 `fallback_enabled` + `fallback_image`；关闭/停用时不下发。 |
| `frontend/src/features/rooms/RoomStartupPanel.vue` | 管理台开关 + 镜像选择 + 只读展示。 |
| `crates/db/src/startup.rs` tests / `crates/api/...` / `crates/device-pki/...` | 旧配置兼容、启用必填、关闭不下发、条目形状单测。 |

> 无需新迁移：fallback 与正常镜像同级保存在已有 `config` JSONB 中，旧行按 serde
> 缺省解析为「未启用」。
