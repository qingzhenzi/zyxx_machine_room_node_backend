# 虚拟机（VM）远程控制

> 控制节点侧最小闭环：控制台下发 VM 命令 → 设备下行（`vm.command`）→ 机器上的 room-agent 执行
> → 结果回写 `machine_command_results` → 控制台轮询展示。
>
> 前置：命令结果表由迁移 **0031** 创建（T1 负责）；权限由迁移 **0032** 播种。
> agent 侧执行逻辑见另一任务的 `qe-agent-node` / room-agent 实现（本仓库只做控制节点侧）。

## 1. 链路

```text
控制台「机器详情 → 虚拟机」面板
  └─ POST /api/v1/admin/machines/{serial}/vm/commands  {op,name?}   (权限 vm:control)
        ├─ INSERT machine_command_results(request_id, serial, kind='vm.command',
        │         payload={request_id,op,args}, status='pending')
        └─ INSERT device_downlink(serial, kind='vm.command', payload=同上)
              └─ device-pki GET /messages/downlink ──► room-agent
                    └─ 执行 list/start/shutdown/reboot/destroy/screenshot
                          └─ 结果回写 machine_command_results.status/result/error
控制台 GET .../vm/commands/{request_id}（1s 轮询，最多 60s）
```

- 下行 kind：**`vm.command`**；payload：
  `{"request_id":"<uuid>","op":"list|start|shutdown|reboot|destroy|screenshot","args":{"name":"..."}}`
  （`list` 的 `args` 为空对象）。
- 结果表（0031）：`machine_command_results(request_id uuid pk, serial text fk, kind text,
  payload jsonb, status pending|ok|error|timeout, result jsonb, error text, created_at, updated_at)`。
- agent 回传约定：`list` → `result={vms:[...]}`（与遥测 `sections.vms` 同结构）；
  `screenshot` → `result={format,image_base64,size_bytes}`；其它命令 `result` 可为空。

## 2. 后端接口（qe-api，管理端 JWT）

### POST `/api/v1/admin/machines/{serial}/vm/commands`

- 权限：`vm:control`（迁移 0032 播种，默认授予 admin；`teacher` 角色也包含）。
- 请求体：

```json
{ "op": "start", "name": "debian12" }
{ "op": "list" }
```

- 校验：`op` 白名单（list/start/shutdown/reboot/destroy/screenshot，大小写不敏感、会 trim）；
  `list` 不接受 `name`；其它命令必须带非空 `name`（≤128 字符）。
- 响应：**202** `{"request_id":"<uuid>"}`。
- 错误：

| 状态 | 场景 |
| --- | --- |
| 400 | op 非法 / name 缺失或超长 / list 带 name / serial 为空 |
| 401 | 未登录 |
| 403 | 缺少 `vm:control` |
| 404 | serial 对应设备不存在 |
| 409/503 | 下行背压或设备当前没有存活证书（pending 行会清理） |

### GET `/api/v1/admin/machines/{serial}/vm/commands/{request_id}`

- 权限：`vm:control`。
- 响应：`{request_id, status, data, error, created_at, updated_at}`
  - `status`: `pending` / `ok` / `error` / `timeout`
  - `data`: agent 回传的 `result`（list 时含 `vms`；screenshot 时含 `image_base64`）
- 不存在（或 serial 与命令不匹配）→ 404；`request_id` 非法 UUID → 400。
- **pending 超过 120s**：GET 时顺手把表更新为 `timeout` 并返回该状态（多副本并发更新幂等）。

## 3. 控制台交互

机器详情页「虚拟机」面板：

- 列表来自最新遥测快照 `sections.vms`（无则空态）；「刷新列表」下发 `op=list`，
  结果覆盖当前列表（以机器真实状态为准）。
- 每台 VM：启动 / 关机 / 重启 / 销毁（前四个带确认弹窗，销毁有不可撤销提示）、截图。
- 命令下发后按 **1s × 60 次** 轮询结果：成功 toast；`status=error` 显示 agent 返回错误；
  `timeout` 提示检查机器在线/agent。
- 截图结果以 `data:image/<format>;base64,...` 在弹窗展示，支持「刷新截图」。
- 无 `vm:control` 权限时只读（不显示操作按钮）。

## 4. 限制与排障

- **机器必须在线且有存活设备证书**，否则命令不会入队（503，pending 行已清理）。
- 命令是「至多一次语义」：下行入队 + agent ACK 才标记投递；执行结果由 agent 回写。
  机器执行中掉线时命令可能停在 pending → 120s 后转 `timeout`，需重新下发。
- 同一台 VM 的并发命令不做互斥（UI 上 `vmBusy` 只防本页面重复点击）；跨操作员并发操作
  由 libvirt 自己报错，结果会体现在 `error`。
- 截图 base64 会占用 JSON 体积（1024 字节级常见 png 也可能几百 KB），大分辨率截图时
  轮询响应会变慢；后续可改为对象存储直传。
- 顺序依赖：先有 0031（表）→ 0032（权限）→ 0033（teacher 角色包含 vm:control）。
- 常见错误：
  - 404：serial 写错 / 机器不存在；
  - 422（旧 agent 遥测无 `sections.vms`）：先让 agent 升级并上报；
  - `status=error` 且 error 含 `libvirt`/`domain`：目标机 libvirt 不可用或 VM 名不存在；
  - 一直 pending：device-pki 下行未送达（设备会话 online 但长连接断开时最多等 2s 兜底轮询）。
