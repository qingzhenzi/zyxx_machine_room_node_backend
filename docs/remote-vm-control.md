# 虚拟机（VM）远程控制

> 控制节点侧控制链路：控制台下发 VM 命令 → 设备下行（`vm.command`）→ 机器上的 room-agent 执行
> → 结果回写 `machine_command_results` → 控制台轮询展示。
>
> 阶段 1 覆盖生命周期/截图；**阶段 1.5** 增加 **底包 + qcow2 差分盘 + 持久化快照 + 磁盘链 + 导出**
> 以及教师「批量下发环境」。磁盘模型与协议细节见
> [`vm-base-image-and-diff-disk.md`](vm-base-image-and-diff-disk.md)。
>
> 前置：命令结果表由迁移 **0031** 创建；权限 `vm:control` 由迁移 **0032** 播种。

## 1. 链路

```text
控制台「机器详情 → 虚拟机」面板
  └─ POST /api/v1/admin/machines/{serial}/vm/commands  {op,args?}   (权限 vm:control)
        ├─ INSERT machine_command_results(request_id, serial, kind='vm.command',
        │         payload={request_id,op,args}, status='pending')
        └─ INSERT device_downlink(serial, kind='vm.command', payload=同上)
              └─ device-pki GET /messages/downlink ──► room-agent
                    └─ 执行 list/start/…/create/snapshot.*/disk.info/export
                          └─ 结果回写 machine_command_results.status/result/error
控制台 GET .../vm/commands/{request_id}（1s 轮询，最多 60s）
```

- 下行 kind：**`vm.command`**；payload：
  `{"request_id":"<uuid>","op":"<op>","args":{...}}`（`args` 已按 op 校验，原样透传给 agent）。
- 结果表（0031）：`machine_command_results(request_id uuid pk, serial text fk, kind text,
  payload jsonb, status pending|ok|error|timeout, result jsonb, error text, created_at, updated_at)`。

## 2. 后端接口（qe-api，管理端 JWT）

### 2.1 单机命令：POST `/api/v1/admin/machines/{serial}/vm/commands`

- 权限：`vm:control`（迁移 0032 默认授予 admin；`teacher` 角色也包含）。
- 请求体兼容两种形式：

```json
{ "op": "start", "name": "debian12" }
{ "op": "create", "args": { "name": "lab-1", "vcpus": 2, "memory_mib": 2048, "disk_gb": 20, "base_volume": "ubuntu-22.04", "start": true } }
```

- 响应：**202** `{"request_id":"<uuid>"}`；`400` 参数非法、`404` 设备不存在、
  `403` 缺权限、`409/503` 下行背压或设备无存活证书（pending 行会清理）。
- **op 白名单与 args**（阶段 1.5）：

| op | args | 说明 |
| --- | --- | --- |
| `list` | 无 | 列出域；data `{vms:[...]}` |
| `start` / `shutdown` / `reboot` / `screenshot` | `{name}` | 生命周期/截图（name 必填） |
| `destroy` | `{name}` | **强制断电**（只停止，不删除定义；可再 start） |
| `delete` | `{name, remove_disks?=false}` | **删除虚拟机**：运行中先 destroy → undefine → 可选删除磁盘文件（并清理空目录）；data `{name, undefine, removed_disks}` |
| `create` | `{name, vcpus=2, memory_mib=2048, disk_gb=20, base_volume?, base_path?, gpu?, start?}` | 用底包创建 qcow2 **差分盘**并 define（可立即启动）；data `{name, disk_path, base, defined, started}` |
| `snapshot.create` | `{name, snapshot, description?}` | 关机状态下创建 qcow2 内部快照 |
| `snapshot.list` | `{name}` | data `{snapshots:[{tag, created_at, vm_size, date}]}` |
| `snapshot.restore` / `snapshot.delete` | `{name, snapshot}` | 关机状态下恢复/删除 |
| `disk.info` | `{name}` | data `{disks:[{path, format, virtual_size, backing_path, backing_format}], chain:[...]}` |
| `export` | `{name, format="qcow2"}` | 导出完整 qcow2；data `{path, sha256, size_bytes}`（上传发布 = 阶段 2） |
| `image.make` | `{vm_name, name, version, platform?, base_volume?, description?}` | **阶段 2**：把关机 VM 导出为镜像并登记（agent 执行导出→分片上传→device-pki register）；data `{image_id, manifest_digest, total_size, chunks}`；pending 超时 600s |

**校验规则**（后端强制，前端同规则预校验）：

- `name` / `snapshot`：`[A-Za-z0-9._-]{1,64}`（防路径注入）；
- `create`：`vcpus` 1..=64、`memory_mib` 256..=262144、`disk_gb` 1..=2048；
  `base_volume` 与 `base_path` 二选一（卷名为 libvirt 存储池卷，`base_path` 必须绝对路径）；
  `gpu` 最多 2 个（PCI 直通格式 `dddd:bb:dd.f`）；未知 args 字段直接 400；
- `snapshot.*` / `export` 要求目标域 **shut off**，运行中由 agent 返回明确错误；
- 阶段 1 的 `{op,name}` 写法继续兼容（等价 `args.name`）。

### 2.2 教师批量下发环境：POST `/api/v1/admin/rooms/{room_id}/vm/environments`

- 权限：`vm:control`。
- 请求体：

```json
{
  "name_prefix": "lab",
  "base_volume": "ubuntu-22.04",
  "vcpus": 2,
  "memory_mib": 2048,
  "disk_gb": 20,
  "auto_start": true
}
```

- 行为：对房间**主设备槽位**（`rooms.device_slot`）+ `room_devices` 子机逐槽位下发
  `op=create`，VM 名 `<name_prefix>-<槽位尾号>`（尾号清洗：`room-101` → `lab-101`）；
  每个槽位插入一条 `machine_command_results(pending)` 并 enqueue 一条 `vm.command`。
- 响应：**202**

```json
{
  "room_id": "…",
  "targets": [
    { "device_slot": "room-101", "serial": "4f40ccef…", "request_id": "…" },
    { "device_slot": "room-102", "serial": null, "request_id": null }
  ]
}
```

  - `request_id/serial = null` 表示该槽位已分配但**没有存活设备证书**，命令未入队（前端显示「未下发」）；
  - 房间不存在 404；房间没有任何绑定槽位 409；规格越界/前缀非法 400。
- 前端拿每个 `request_id` 用 2.1 的 GET 接口轮询状态。

### 2.3 查询结果：GET `/api/v1/admin/machines/{serial}/vm/commands/{request_id}`

- 权限：`vm:control`；`(serial, request_id)` 不存在 404；`request_id` 非 UUID 400。
- 响应：`{request_id, status, data, error, created_at, updated_at}`；
  `status`: `pending` / `ok` / `error` / `timeout`；`data` 为 agent 回传 `result`。
- **pending 超过 120s**：读取时顺手把表更新为 `timeout` 并返回（多副本并发更新幂等）。

### 2.4 「从 VM 制作镜像」链路（阶段 2）

控制台点「制作镜像」后，控制面下发 `vm.command {op:"image.make", args}`（权限 `vm:control`），
agent 执行「导出 qcow2 → 32 MiB 分片 → 逐片 presign/PUT → register」，具体协议见
[`vm-base-image-and-diff-disk.md`](vm-base-image-and-diff-disk.md) §8.5。

**device-pki 设备 mTLS 端点**（agent 调用，身份来自设备证书）：

| 方法/路径 | 请求体 | 响应 | 错误 |
| --- | --- | --- | --- |
| `POST /machine/images/presign-upload` | `{sha256, size_bytes}` | `{object_key:"objects/<sha256>", upload_url, expires_at, already_exists}` | 400（sha/大小非法）、401/403（证书无存活注册）、503（未配置 S3） |
| `POST /machine/images/register` | `{name, version, platform, kind:"vm_disk", format:"qcow2", compression:"none", total_size, parts:[{sha256,size}], annotations?}` | `{image_id, manifest_digest, status:"manifest_ready"}` | 400（参数/分片对象缺失或大小不符）、409（同名/同版本/同平台已存在）、503（S3 不可用） |

register 校验（服务端不信任 agent）：

- `parts` 1..=2048，单 part 1..=64 MiB，`sum(part.size)==total_size`（≤32 GiB），sha256 必须 64 位小写 hex；
- 每个 part 的 `objects/<sha256>` 必须已上传，且服务端 `HEAD content-length == part.size`；
- `kind=vm_disk` / `format=qcow2` / `compression=none`（其它 400）；
- `name/version/platform` 复用 `qe_domain::storage` 校验；`name` 只允许小写字母/数字/`-_.`；
- annotations 保留调用方字段，并强制 `source="vm-export"` + `serial=<设备证书 serial>`；
- 服务端构造并 Ed25519 签名 manifest（`manifests/<digest>.json`、`signatures/<digest>.sig`），
  写 `image_artifacts`（status=`manifest_ready`，created_by=NULL）+ `image_chunks`；
  `(name,version,platform)` 重复 → 409；失败时清理本次写入的 manifest/签名对象与记录。

**控制台编排**：`image.make` 成功后（data 含 `image_id`/`manifest_digest`）前端串行调用
`POST /admin/images/{id}/verify` → `POST /admin/images/{id}/publish`（发布通道 dev/test/prod），
展示 verify 检查项与最终状态，成功后可从 `/images` 分配/发布。

**边界**：

- VM 必须 **shut off**（运行中按钮禁用；agent 侧也会拒绝）；
- 导出/上传/登记耗时较长：`image.make` 的 pending 超时阈值为 **600s**（普通命令 120s）；
- 分片 32 MiB 是 agent 侧切割大小，远低于 `get_bytes` 64 MiB 上限，复用现有 `verify` 全链路；
- 同一 sha 对象内容寻址，`presign-upload` 命中 `already_exists=true` 时 agent 跳过上传；
- 若上传/register 中途失败，agent 重试即可（presign/register 幂等；同名镜像重复 register 返回 409）。

## 3. 控制台交互

机器详情页「虚拟机」面板（`vm:control` 才显示操作，否则只读）：

- 列表来自最新遥测 `sections.vms`；「刷新列表」下发 `list`，结果覆盖展示（以机器真实状态为准）。
- 每台 VM：启动 / 关机 / 重启 / 销毁 / 截图 / **快照** / **磁盘链** / **导出**。
- **新建虚拟机**：名称、vCPU、内存 MiB、磁盘 GiB、系统底包（从已发布 `vm_disk` 镜像选
  `<name>-<version>` 卷名）、是否启动、GPU 直连（可选）。
- **快照抽屉**：创建 / 列表 / 恢复 / 删除（恢复/删除有确认框；要求关机状态）。
- **磁盘链**：`disk.info` 展示 底包 → 差分盘 → 快照 的关系与每个磁盘的 backing 信息。
- **导出**：展示机器本地导出路径与 sha256，并注明「上传为镜像 = 阶段 2」。
- 命令下发后 1s × 60 次轮询；成功 toast、agent 错误/超时提示；启停/创建后自动刷新列表。
- **制作镜像**：关机 VM 行内「制作镜像」按钮（需 `vm:control` + `image:manage`）→ 填镜像名/版本/平台/发布通道 →
  `image.make`（最长等 10 分钟）→ 自动 verify + publish，弹窗展示三步状态与 verify 结果，成功后链接 `/images`。

教师工作台新增「下发环境配置」卡：选房间 + 底包 + 规格 → 批量下发 →
展示每槽位的 VM 名预览、request_id 与轮询状态（已创建/失败/超时/未下发）。

## 4. 限制与排障

- **机器必须在线且有存活设备证书**，否则单机命令 503、批量该槽位 `request_id=null`（pending 行已清理）。
- 命令是「至多一次 + 结果回传」语义：下行入队 + agent ACK 才标记投递；机器执行中掉线时
  命令可能停在 pending → 120s 后转 `timeout`，需重新下发。
- **快照要求关机**：运行中创建/恢复/删除快照会被 agent 拒绝（避免 qcow2 元数据不一致）。
- **底包来自既有镜像管道**：`image.pull` → `importer/vm.rs` 落成 libvirt 池卷
  `<image name>-<version>`；控制台只引用卷名，不复制数据。
- **导出上传属阶段 2**：当前 `export` 只在机器本地产出完整 qcow2 + sha256；
  「上传对象存储 → 单分片 manifest → 发布镜像」在阶段 2 接入。
- 同一台 VM 的并发命令不做互斥（UI 的 `vmBusy` 只防本页面重复点击）；跨操作员并发由 libvirt 报错，
  结果体现在 `error`。
- 常见错误：
  - 404：serial 写错 / 机器不存在；
  - 422 或 `data=null`：旧 agent 无 `sections.vms`，先升级 agent 并上报；
  - `status=error` 且含 `libvirt`/`domain`：目标机 libvirt 不可用或 VM 名不存在；
  - `snapshot.*` 报运行中：先 `shutdown`；
  - 一直 pending：device-pki 下行未送达（设备 online 但长连接断开时最多等 2s 兜底轮询）。
