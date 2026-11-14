# 教师端镜像制作工作台契约（Image Studio v1）

> 2026-11-13。承接 [image-build-lifecycle.md](image-build-lifecycle.md) 与
> [image-lifecycle-multiteacher.md](image-lifecycle-multiteacher.md)。
> 目标：教师不 SSH、不手工 virsh，在教师端完成「选机器/底包 → 构建 VM → VNC 装系统 →
> 结束制作 → 分片上传 → 校验 → 发布 → 设为房间默认底包」。

## 1. 用户体验流程

```
镜像库（/images）右上 [新建镜像]
  → 步骤 1 选择构建目标
      房间（默认当前房间）→ 机器（房间内在线 agent，含主设备/子机）
      基础镜像（默认 = 房间默认底包；可选我的/公共 published 镜像）
      镜像名/版本/平台/说明，vCPU/内存/磁盘
  → 步骤 2 准备（服务端构建会话）
      确保基础镜像已分配到房间并完成 image.pull（显示 transfer.progress）
      自动创建并启动构建 VM（名称 qe-build-<8hex>，差分盘 backing=底包）
  → 步骤 3 操作（构建 VM 就绪）
      教师端内嵌 noVNC 控制台（键鼠注入），在 VM 内安装/配置系统
  → 步骤 4 结束制作
      关机（优雅 shutdown，超时自动 destroy）→ image.make 导出 qcow2
      → 32MiB 分片上传（S3 内容寻址）→ verify → publish(dev)
  → 完成
      可 [设为当前房间默认底包]；跳转镜像库
```

约束：

- 构建 VM 必须由**服务端会话**统一创建/回收，教师端关闭后仍可从「进行中的制作」恢复；
- 基础镜像必须在**目标机器**上完成 import 后才能创建差分盘（会话负责等待 transfer）；
- 构建 VM 关机后 `image.make` 复用现有「制作工单」身份桥（draft + build_request）；
- 默认发布通道 `dev`；发布完成后才能被分配/下发。

## 2. 服务端模型（迁移 0039）

```sql
-- 房间默认底包（底包 + 增量包；增量包本期只做字段与拉取，应用逻辑 P3）
CREATE TABLE room_image_defaults (
  room_id UUID PRIMARY KEY REFERENCES rooms(id) ON DELETE CASCADE,
  base_image_id UUID NOT NULL REFERENCES image_artifacts(id) ON DELETE RESTRICT,
  delta_image_id UUID REFERENCES image_artifacts(id) ON DELETE SET NULL,
  updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 构建会话
CREATE TABLE image_build_sessions (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
  machine_serial TEXT NOT NULL,
  build_vm_name TEXT NOT NULL,                 -- qe-build-<8hex>
  base_image_id UUID NOT NULL REFERENCES image_artifacts(id),
  delta_image_id UUID REFERENCES image_artifacts(id),
  base_transfer_id UUID,                       -- 等待的 image.pull transfer
  name TEXT NOT NULL, version TEXT NOT NULL, platform TEXT NOT NULL,
  vcpus INT NOT NULL, memory_mib INT NOT NULL, disk_gb INT NOT NULL,
  description TEXT,
  state TEXT NOT NULL,       -- preparing | creating | ready | finishing | making | done | failed | cancelled | expired
  error TEXT,
  create_request_id UUID, shutdown_request_id UUID, destroy_request_id UUID,
  make_request_id UUID,
  image_id UUID REFERENCES image_artifacts(id) ON DELETE SET NULL,  -- image.make 产出的镜像
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  finished_at TIMESTAMPTZ
);
CREATE INDEX image_build_sessions_user_idx ON image_build_sessions (user_id, created_at DESC);
CREATE INDEX image_build_sessions_active_idx ON image_build_sessions (state) WHERE state NOT IN ('done','failed','cancelled','expired');
```

状态机（`GET` 时由服务端幂等推进，避免依赖客户端定时器）：

```
preparing ──(base transfer completed)──> creating ──(vm create result ok)──> ready
ready ──finish──> finishing ──(VM shut off)──> making ──(image.make result ok)──> done
任意非终态 ──cancel / stale(>2h)──> cancelled / expired（destroy + delete VM，删草稿）
任意步骤失败 ──> failed（保留 error，VM 尽力回收）
```

## 3. API（qe-api，JWT，教师权限 vm:control + image:manage）

### 3.1 房间默认底包

- `GET /api/v1/admin/rooms/{room_id}/image-default` → `{base_image: ImageArtifactView|null, delta_image: ...|null}`
- `PUT /api/v1/admin/rooms/{room_id}/image-default` body `{base_image_id, delta_image_id?}`
  - 校验镜像 published 且对当前用户可见（owner/public）；写 `image_assignments`（房间级）并保证 transfer；
  - 权限：`vm:control`（教师可设自己房间）+ owner-or-admin 校验；
- `/agent/startup` 集成：把房间默认底包（与可选增量包）追加到 `images[]`（若未配置 startup 配置也生效），
  保证 agent 连上后自动拉取默认底包；增量包本期只拉取不自动挂载（P3）。

### 3.2 构建会话

- `POST /api/v1/admin/image-builds`
  ```json
  {"room_id":"…","machine_serial":"…","base_image_id":"…","name":"lab-base",
   "version":"1.0.0","platform":"linux/amd64","vcpus":2,"memory_mib":2048,"disk_gb":40,
   "description":"…"}
  ```
  - 校验：机器属于该房间且在线；基础镜像 published/可见；目标机器有存活证书；同名预检 409；
  - 写入 session(state=preparing)，确保房间分配 + 触发 image.pull，记录 base_transfer_id；
  - 返回完整 session（含 steps 与 base_transfer 进度）。
- `GET /api/v1/admin/image-builds/{id}`（owner/admin）
  - 幂等推进状态机：
    - preparing：查询 transfer；completed → 发送 `vm.command create`（base_volume=`<base.name>-<base.version>`，
      `start=true`，name=build_vm_name）→ creating；
    - creating：查询 `machine_command_results`；ok → ready；error → failed；
    - finishing：查询 shutdown/destroy 结果；VM off → 发送 `image.make`（args 含 vm_name/name/version/platform/description）
      → making；60s 未关机自动 destroy；
    - making：查询 image.make 结果；ok → done + image_id；error/timeout → failed；
  - 返回字段：`state, error, steps[], base_transfer{status,progress}, vm{name,state}, image_id, commands{...}`。
- `GET /api/v1/admin/image-builds?active=1` → 当前用户未结束会话（教师端「进行中的制作」恢复）。
- `POST /api/v1/admin/image-builds/{id}/finish` → 仅在 ready 时允许；置 finishing 并发 shutdown。
- `POST /api/v1/admin/image-builds/{id}/cancel` → destroy VM（若存在）→ `vm.command delete`（remove_disks=true）
  → 若已有 make 草稿则删除 → cancelled。
- 后台清理 worker（60s/轮）：非终态且 `updated_at < now()-2h` → 回收 VM + 置 expired；记录 image_events。

### 3.2.1 会话响应形状（教师端直接消费）

```json
{
  "id": "uuid",
  "state": "preparing|creating|ready|finishing|making|done|failed|cancelled|expired",
  "error": null,
  "room_id": "uuid",
  "machine_serial": "…",
  "build_vm_name": "qe-build-a1b2c3d4",
  "base_image": {"id":"…","name":"real-debian-13","version":"0.1.1","platform":"debian/amd64",
                 "total_size":2148073472,"status":"published"},
  "name": "lab-base", "version": "1.0.0", "platform": "linux/amd64",
  "vcpus": 2, "memory_mib": 2048, "disk_gb": 40, "description": "…",
  "steps": [
    {"key":"base_pull","label":"拉取系统底包","status":"ok|running|pending|error","detail":"100%"},
    {"key":"create_vm","label":"创建构建虚拟机","status":"running","detail":"…"},
    {"key":"console","label":"远程操作","status":"pending","detail":"…"},
    {"key":"shutdown","label":"关机","status":"pending","detail":""},
    {"key":"upload","label":"导出并分片上传","status":"pending","detail":""}
  ],
  "base_transfer": {"id":"uuid","status":"pending|running|completed|failed","progress":0,"error":null},
  "image_id": null,
  "created_at": "…", "updated_at": "…", "finished_at": null
}
```

### 3.3 权限与可见性

- 会话只能由创建者（或 admin）查看/结束/取消；
- 目标机器必须属于 `room_id`（room.device_slot 或 room_devices）；
- 基础镜像：published，且 owner=当前用户 / visibility=public；
- 构建产出的镜像 owner=创建者（沿用 image.make 工单桥）。

## 4. 教师端（Tauri）前端

### 4.1 路由与入口

- `/images` 右上按钮改为 **[新建镜像]**（打开 `/images/build`），保留「从已有 VM 制作」为次级入口；
- 新增 `/images/build`（BuildStudio 向导，单页即可）：
  - Step 1 表单：房间 → 机器（在线，默认当前房间主设备）→ 基础镜像（默认房间默认底包，标记「房间默认」）→
    名称/版本/平台/说明/规格；
  - Step 2 准备：会话轮询（transfer 进度 + 创建 VM 阶段）；
  - Step 3 操作：内嵌 `VncConsole`（noVNC），提示「在 VM 里安装系统；完成后点结束」；
  - Step 4 结束/上传：关机 → image.make → verify（长超时）→ publish(dev) → 完成；
  - 完成页：`设为 {room} 默认底包` 按钮（PUT image-default）+ 查看镜像。
- 镜像库顶部显示当前房间默认底包（可切换），新镜像 published 后可直接设默认。

### 4.2 VNC 控制台（教师端）

- 新增依赖 `@novnc/novnc@1.7.0`（与管理台一致）；
- 新增 `src/components/VncConsole.vue` + `src/lib/vnc.ts`（移植管理台 `vncSession.ts` 的 WS URL 拼接/状态逻辑）：
  - 调 `POST /api/v1/admin/machines/{serial}/vnc/session {vm}` 拿 `ticket/password`；
  - WS：`ws(s)://<control-host>/api/v1/admin/machines/{serial}/vnc/ws?session=&ticket=`；
  - 状态：申请中/连接中/已连接/已断开/失败；断开可重连；
- Tauri CSP `connect-src` 需允许控制面 http/ws（当前写死 `http://127.0.0.1:8080 http://192.168.8.231:8080`，
  增加 `ws://127.0.0.1:8080 ws://192.168.8.231:8080 http://*:8080 ws://*:8080`）。

## 5. 增量包（底包+增量包）设计说明

- 本期：`room_image_defaults.delta_image_id` 仅存储 + 在 `/agent/startup` 一并下发拉取；
  启动配置的 VM 仍以 `base_volume` = 底包卷名创建差分盘；
- P3：增量包定义为「相对底包的 qcow2 差分」；agent 创建 VM 时先以底包为 backing 生成盘，
  再用 `qemu-img rebase/commit` 应用增量（或直接把增量作为 backing 链第二层）；需新增镜像层元数据
  `parent_manifest_digest` 与 agent 端 apply 逻辑。设计保留字段，不做假实现。

## 6. 验收

1. 服务端：`cargo check/test`（qe-db/qe-api/qe-device-pki）；构建会话状态机单测；
2. 教师端：`pnpm type-check/lint/build`；
3. 真机 E2E（192.168.8.141 / dev-b104）：
   - 房间默认底包设为 `real-debian-13:0.1.1`；
   - 教师端/API 新建构建会话（目标 dev-b104 主设备，底包 real-debian-13）→ 状态到 ready，
     `virsh list` 出现 `qe-build-*` 运行；
   - VNC：`POST /vnc/session` 返回 ticket，WS 代理可握手（教师端组件 type-check/build 通过）；
   - finish → 关机 → image.make → verify → publish → `image_id` 出现在会话；
   - 设置房间默认底包成功；`GET /agent/startup` 返回默认底包 image；
   - 取消会话能回收 VM；会话只能本人可见。

## 7. 服务端实现备注（2026-11-13）

以下为实现取舍，不改变 §2/§3 的协议字段：

- **迁移 0039**：`room_image_defaults` + `image_build_sessions`，全部 `IF NOT EXISTS` 幂等；
  `image_build_sessions.state` 带 CHECK 约束（9 个状态）。
- **会话状态机**在 `GET /admin/image-builds/{id}`（以及 `GET ?active=1`）时幂等推进，
  多副本用 `UPDATE ... WHERE state = ANY($from) RETURNING` 的 CAS 语义，只有一个请求生效，
  其余请求重新读取；不依赖客户端定时器。
- **状态推进细节**：
  - `preparing`：轮询 `base_transfer_id` 对应 transfer；`completed` → 下发
    `vm.command create`（`base_volume=<base.name>-<base.version>`, `start=true`,
    `name=qe-build-<8hex>`）并进入 `creating`；`failed/expired` → `failed`；
  - `creating`：读 `machine_command_results`；`ok` → `ready`，其余终态 → `failed`；
  - `finishing`：`shutdown` 结果 `ok` → 下发 `image.make`；`shutdown` 失败或进入
    finishing 超过 60s → 自动 `destroy`，`destroy` `ok` → 下发 `image.make`，
    `destroy` 失败 → `failed`；
  - `making`：读 `image.make` 结果；`ok` → `done`（`image_id` 在下发 make 时即记录为
    服务端预创建的 draft id）；`error/timeout` → `failed`。
- **base_transfer**：会话创建时由内部 `ensure_image_assigned`（复用 `assign_room`
  的 upsert assignment + pending transfer SQL，不经过 API 权限）保证；已有
  `pending/running/completed` transfer 时复用最新一条，只有 `failed/expired` 才新建。
- **image.pull 目标**：构建会话把 `image.pull` 投给**目标机器的设备槽位**
  （`room.device_slot` 或 `room_devices`，可能是子机）；房间默认底包仍投给房间主槽位。
- **同名 409**：创建会话时预检两层 —— 个人命名空间下同 `(name,version,platform)` 的
  非 deprecated/revoked 镜像；以及当前教师已存在的同名进行中会话。
- **`GET /admin/image-builds?active=1`** 返回 `BuildSessionReply[]`（数组，最新在前，
  最多 20 条），每项形状与单会话 GET 一致。
- **cancel**：非终态均可取消；`destroy` + `delete(remove_disks=true)` 尽力而为，
  若已有未消费的 draft（`image.make` 已下发但还没 register）则删除；终态重复 cancel
  对 `cancelled/expired` 幂等返回当前会话，对 `done/failed` 返回 409。
- **后台回收**：随 qe-api 启动的 60s worker，非终态且 `updated_at` 超过 2h →
  `expired`，尽力 destroy/delete 构建 VM 并写 `image_events(actor_kind=system)`。
- **`/agent/startup` 合并**：把房间默认底包/增量包追加进 `images[]`（按 image_id 去重，
  只下发 published + manifest 完整）；**无 startup profile 也生效**：此时
  `enabled=true`、`revision=默认底包行 updated_at 的 Unix 秒（恒 >0）`、auto 全开、
  `vms=[]`。有 profile 且 `enabled=false` 时尊重显式禁用（不下发默认底包）。
- **审计/事件**：会话 create/finish/cancel 写 `audit_logs`（resource_type=
  `image_build_session`），done/failed 在状态实际变化时写一条；默认底包变更同时写
  `image_events(room.default.update)`；超时回收写 `image_events(build.session.expired)`。

---

# 附录 A：ISO 安装 / Windows 支持（v2，2026-11-14）

## A.1 目标

教师端「新建镜像」支持两种来源，共用同一套构建会话与后续 image.make → verify → publish：

1. `source=base`：用已发布底包创建差分盘（现状）；
2. `source=iso`：用目标机器上已有的安装 ISO 创建安装 VM（SATA 磁盘 + CD-ROM 启动），
   教师通过内嵌 noVNC 安装 Windows/Linux，安装完成后「弹出 ISO → 从硬盘启动 → 结束制作」。

## A.2 完整状态机（image_build_sessions.state）

```text
base 模式:
  preparing ──base transfer completed──> creating ──create ok──> ready
iso 模式:
  preparing ──iso.list 校验 ISO 存在──> creating ──create ok──> installing
  installing ──[安装完成，弹出 ISO]──> ejecting ──eject ok──> ready
全部模式:
  ready ──finish──> finishing ──VM 关机(destroy 终态)──> making ──image.make ok──> done
  任意非终态 ──cancel / >2h──> cancelled / expired
  任一步失败 ──> failed（保留 error / steps 中标记失败步骤；尽量回收 VM）
```

- `finish` 仅允许 `ready`；iso 模式必须先完成 `ejecting`，UI 引导「先点安装完成，弹出 ISO」；
- `installing` 是长驻交互态：教师通过 VNC 操作，会话 GET 返回 steps 供 UI 展示；
- `ready` 后可选择「重启进入系统」（复用 `vm.command reboot`）再结束制作；
- 取消/失败/超时都会 destroy + delete(remove_disks) 构建 VM；完成后自动回收。

## A.3 新增/扩展接口

### A.3.1 vm.command（qe-api 白名单 + agent 实现）

| op | args | data | 说明 |
|---|---|---|---|
| `iso.list` | — | `{isos:[{name,size_bytes}]}` | 列出目标机 `LIBVIRT_ISO_DIR`（默认 `/var/lib/libvirt/images/iso`）下 ISO；name 不含路径分隔符 |
| `eject` | `{name}` | `{name, ejected:true}` | 弹出 CD-ROM（`virt-crate eject_cdrom`），装完系统后从硬盘启动 |
| `create`（扩展 args） | 原字段 + `iso_name?`、`disk_bus?`(virtio/sata，默认 virtio)、`nic?`(none/user/default)、`nic_model?`(e1000/virtio) | 原字段 | 有 `iso_name` 时挂 CD-ROM 并按 cdrom 优先启动；Windows 建议 `disk_bus=sata`、`nic=none`（离线安装） |

安全：`iso_name` 只允许文件名（拒绝 `/\`、`..`），agent 解析为 `<LIBVIRT_ISO_DIR>/<name>` 并校验存在；
ISO 列表只返回文件名/大小，不返回宿主路径。

### A.3.2 image-builds

`POST /admin/image-builds` body 扩展：

```json
{ "room_id":"…", "machine_serial":"…",
  "source": "base" | "iso",
  "base_image_id": "…（source=base 必填）",
  "iso_name": "Win10_LTSC_2021.iso（source=iso 必填）",
  "disk_bus": "virtio | sata（可选；iso 默认 sata）",
  "nic": "none | user | default（可选；默认 none）",
  "name":"win10-ltsc","version":"0.1.0","platform":"windows/amd64",
  "vcpus":4,"memory_mib":4096,"disk_gb":40,"description":"…" }
```

- `preparing`（iso）：先下发 `vm.command iso.list`（记录 `iso_check_request_id`），确认 ISO 存在后再创建 VM；
  ISO 不存在 → `failed`（error 明确提示）；
- `creating`：`vm.command create` args 注入 `iso_name/disk_bus/nic/start=true`；
- `POST /admin/image-builds/{id}/eject-iso`：仅 `installing` 可用，置 `ejecting` 并下发 `vm.command eject`；
  eject ok → `ready`（`media_ejected=true`）；
- `GET /admin/image-builds/{id}` 的 steps（iso 模式）：
  `iso_check`（校验安装 ISO）→ `create_vm` → `install`（远程安装/配置）→ `eject_iso`（弹出 ISO）→
  `shutdown` → `upload`；
- 响应增加 `source_mode/iso_name/disk_bus/nic/media_ejected`。

## A.4 Windows 设备兼容

- **磁盘总线**：`virt-crate` VmSpec 增加 `disk_bus`（Virtio 默认 / Sata）；SATA 目标名 `sda/sdb…`，
  CD-ROM 目标名避让（`sdz` 或按序号）；Windows 安装盘自带 AHCI 驱动，无需 virtio-win；
- **网卡**：可选 `user`/`default`，Windows 兼容模型 `e1000`（默认 none，离线安装避免微软账号强制）；
- 显示：现有 VNC + tablet 指针即可，Windows 桌面/键鼠已验证；
- 后续可选：virtio-win 驱动注入（性能更好），本期 SATA 优先。

## A.5 教师端交互

- 新建镜像 Step1 增加来源 Tab：`已有镜像底包` / `从 ISO 安装`；
  ISO 模式：选择目标机器 → 拉取 `iso.list` → 选择 ISO → 自动预填（win10/ltsc → windows/amd64、4 vCPU/4GB/40GB、sata、none）；
- Step2/3：`installing` 状态展示内嵌 VNC + 明确提示「在 VM 内完成系统安装，完成后点『安装完成，弹出 ISO』」；
  `ejecting` 展示进度；`ready` 展示 [重启进入系统]（调用 vm.command reboot）与 [结束制作并保存]；
- 结束后流程不变：关机 → image.make → verify → publish dev → 设为房间默认底包；
- 任务中心/`?active=1` 恢复支持所有新状态。

## A.6 验收

1. 服务端/agent 单测：ISO 名称解析与目录限制、disk_bus XML、iso.list/eject/create(iso) 状态机；
2. 真机：目标机 `iso.list` 列出 Win10 LTSC ISO → id=iso 会话 → `installing`（VNC 显示 Windows 安装界面）
   → 取消可回收；eject 流程可用；
3. Windows 底包复用：把已装好的 win10-ltsc 导出为 `win10-ltsc:0.1.0` 并发布；base 模式 + `disk_bus=sata`
   创建构建 VM → VNC 直接进 Windows 桌面 → 可键鼠交互 → 结束制作/取消；
4. 教师端 type-check/lint/build/test 通过；管理台兼容（未知字段忽略）。

## A.7 服务端实现备注（v2，2026-11-14）

- **迁移 0040** `0040_image_build_iso.sql`：`image_build_sessions` 增列
  `source_mode`（CHECK base|iso）、`iso_name`、`disk_bus`（CHECK virtio|sata，默认 virtio）、
  `nic`（CHECK none|user|default，默认 none）、`media_ejected`（默认 false）、
  `iso_check_request_id`、`eject_request_id`；`base_image_id` **DROP NOT NULL**（iso 模式为 NULL）；
  state CHECK 重建为包含 `installing` / `ejecting` 的 v2 约束。全部幂等（已用真实库事务回滚验证）。
- **`POST /admin/image-builds`**：`source` 缺省 `base`（向后兼容 v1 请求）；`base_image_id`
  变为可选 —— base 必填、iso 忽略；`iso_name` base 传非空 → 400、iso 必填；
  `disk_bus` 缺省按来源（base→virtio、iso→sata），`nic` 缺省 none；iso 模式拒绝 `delta_image_id`。
- **iso 模式 preparing**：创建会话时**立即**下发 `vm.command iso.list` 并记录
  `iso_check_request_id`（设备离线/无存活证书 → 请求直接失败，不建会话）；GET 时轮询结果：
  `isos[].name` 中存在 `iso_name` → 下发 `create`（args 注入 `iso_name/disk_bus/nic/start=true`）
  并进入 `creating`；ISO 不存在/命令失败 → `failed`（error 明确提示）。
  （兜底：若历史 iso 会话缺 `iso_check_request_id`，GET 会补发一次 iso.list。）
- **状态机**：`creating` 命令 ok 后按来源进入 `ready`（base）或 `installing`（iso）；
  `installing` 只由教师 VNC 操作，GET 原样返回 steps（不自动推进）；
  `POST /image-builds/{id}/eject-iso`（仅 iso + installing）→ `ejecting` 并下发
  `vm.command eject`；GET 轮询 eject 结果，ok → `ready` + `media_ejected=true`，失败 → `failed`。
- **finish**：仍仅 `ready`；iso 模式额外要求 `media_ejected=true`，否则 409
  「请先完成『安装完成，弹出 ISO』」。
- **cancel / 超时回收**：`installing`/`ejecting` 均为非终态，cancel 可用；60s worker 的
  >2h 回收扫描按「非终态」判定，天然覆盖新状态（destroy + delete + expired）。
- **steps**：iso 模式为 `iso_check → create_vm → install → eject_iso → shutdown → upload`
  （base 模式保持 v1 的 5 步不变）；`installing` 时 install=running、eject_iso=pending
  且 detail 提示「安装完成后点『安装完成，弹出 ISO』」。
- **响应**：会话回复新增 `source_mode/iso_name/disk_bus/nic/media_ejected`；iso 会话的
  `base_image` / `base_transfer` 为 `null`。
- **vm.command 扩展**：`iso.list`（无参，data `{isos:[{name,size_bytes}]}`）、`eject`
  （`{name}`）、`create` 新 args `iso_name/disk_bus/nic/nic_model`；服务端校验
  iso_name 仅文件名（拒 `/` `\` `..`）、disk_bus∈{virtio,sata}、nic∈{none,user,default}、
  nic_model∈{e1000,virtio}，非法/未知字段 400。管理台可直接用现有
  `POST /admin/machines/{serial}/vm/commands` 调 `iso.list` 查看目标机 ISO。
- **本期不做**：ISO 上传 API（要求 ISO 已由运维/管理台放到目标机 `LIBVIRT_ISO_DIR`，默认
  `/var/lib/libvirt/images/iso`）；Windows 驱动注入（SATA 优先，virtio-win 后续）。

## A.8 实现备注（2026-11-14 联调补充）

- **构建 VM 显示设备**：显示开启时除 `<input type='tablet' bus='usb'/>` 还必须挂
  `<input type='keyboard' bus='usb'/>`；否则 Windows 安装程序/登录界面无法键盘输入。
- **noVNC 缩放**：`@novnc/novnc@1.7.0` 的 RFB 构造函数不消费 `scaleViewport`/`viewOnly`
  选项，必须在实例上显式 `rfb.scaleViewport = true`（内部触发 `display.autoscale`）。
  不要用 CSS 直接缩放 canvas：指针坐标映射会错位，表现为按钮点不到。
- **VNC 容器**：教师端构建工作台固定高度 `h-[520px]`，组件按容器等比缩放；
  另提供 Ctrl+Alt+Del 按钮（Windows 登录界面需要）。
- **ISO 上传**：本期只支持目标机 ISO 目录已有文件；`iso.list` 只读、不暴露宿主路径。
- **Windows 镜像导出**：大磁盘（~10GB）按当前 S3 分片速度约 25–35 分钟，且 image.make pending
  超时 600s 会先超时；Windows 制作建议后续启用分片并发/压缩或放宽该 op 的超时。

---

# 附录 B：ISO 资产库与自动分发（v3，2026-11-14）

目标：教师端不再手工把 ISO 拷到目标机。ISO 作为平台一等资产：**上传/预设 → ISO 库 → 构建时自动分发到目标 agent → 安装**。

## B.1 数据模型（migration 0041）

```sql
CREATE TABLE install_isos (
  id uuid PRIMARY KEY,
  filename text NOT NULL,            -- 安全文件名，目标机 ISO 目录最终文件名（唯一）
  display_name text NOT NULL,
  description text,
  size_bytes bigint NOT NULL DEFAULT 0,
  sha256 text,
  status text NOT NULL DEFAULT 'uploading'
    CHECK (status IN ('uploading','ready','failed')),
  storage_key text,                  -- S3: isos/<id>/<filename>
  is_preset boolean NOT NULL DEFAULT false,
  uploader_user_id uuid REFERENCES users(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  uploaded_at timestamptz
);
CREATE UNIQUE INDEX install_isos_filename_key ON install_isos (lower(filename));
```

- `is_preset=true` 表示系统预设 ISO（seed 目录扫描/管理员上传），所有教师可见；
- 非 preset 的 ISO 仅上传者与 admin 可见/可用；
- filename 必须匹配 `[A-Za-z0-9._-]{1,128}` 且以 `.iso` 结尾（大小写不敏感）；
- 本期不实现分片上传：单次 HTTP PUT 流式接收（上限 16 GiB），服务端落临时文件→校验大小/SHA→上传 S3→置 ready。

## B.2 API（admin，需 image:manage；读取需 image:read）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/admin/isos?status=ready` | 列表：id/filename/display_name/size_bytes/sha256/is_preset/uploader/created_at |
| POST | `/admin/isos` | body `{filename, display_name?, description?, sha256?}` → 建 uploading 记录，返回 `{id, upload_url:"/api/v1/admin/isos/{id}/content"}` |
| PUT | `/admin/isos/{id}/content` | 原始 body 流式写入；仅 uploading 状态；边写边算 SHA-256；大小上限 16GiB；完成后上传 S3（`isos/<id>/<filename>`）并置 ready（返回记录） |
| DELETE | `/admin/isos/{id}` | 删除记录 + S3 对象；有非终态会话引用 → 409 |
| POST | `/admin/isos/seed/scan` | 扫描 `ISO_SEED_DIR`（默认 `/var/lib/qe-control/iso-seeds`）下的 `.iso`，计算大小/SHA，已存在同 filename+sha 跳过，否则上传 S3 并登记 preset ISO；返回 `{scanned, imported, skipped, failed:[...]}` |

- qe-api 启动时后台执行一次 seed 扫描（失败只 warn，不阻塞启动）；
- 预设 ISO 的 filename 冲突规则：同 filename 且 sha 相同则跳过；不同 sha 同名 → 记录失败并在返回中列出。

## B.3 构建会话扩展

`POST /admin/image-builds` 新增 `iso_id`（与 `iso_name` 二选一）：

- `iso_id`：ISO 库资产（推荐）。服务端校验 ready、可见性；filename 取库记录；preparing 阶段：
  1. `iso_check`：校验 ISO 资产 + 生成一次性下载 URL（presigned GET，TTL ≥ 1h）；
  2. 入队 `vm.command iso.pull {name, url, sha256, size_bytes}` 记录 `iso_pull_request_id`；
  3. 轮询结果为 ok → 入队 `create`；失败 → `failed`；
  4. steps：`iso_check → iso_distribute → create_vm → install → eject_iso → shutdown → upload`；
- `iso_name`：保留 v2 兼容（目标机已有 ISO），steps 仍为 6 步。
- 会话回复增加 `iso_id`；迁移 0041 增加 `iso_id uuid NULL`、`iso_pull_request_id uuid NULL`。
- 分发是幂等的：目标机已有同名且 SHA 匹配的 ISO 时 agent 返回 `skipped=true`，会话直接进入创建 VM。

## B.4 agent `iso.pull`

`vm.command op=iso.pull args {name,url,sha256?,size_bytes?}`：

- name 规则同 create（仅文件名），目录 `LIBVIRT_ISO_DIR`；
- 目标文件已存在：若提供 sha256 且匹配（或未提供 sha 但 size 匹配）→ 返回 `{name,size_bytes,sha256,pulled:false,skipped:true}`；
- 否则流式下载到 `<dir>/.<name>.part`，限制大小（≤16GiB）、超时 ≥1800s；校验 size/sha 后原子 rename；
- 返回 `{name,size_bytes,sha256,pulled:true,skipped:false}`；失败清理 .part 并给出明确错误。

## B.5 教师端

- 「从 ISO 安装」优先展示 **平台 ISO 库**（GET /admin/isos?status=ready）：预设徽标、大小、上传时间；
- 「上传 ISO」：文件选择 → POST /admin/isos → XHR PUT 到 upload_url（显示进度/速度/失败重试）→ 刷新列表；
- 选择库内 ISO → 创建会话带 `iso_id`；preparing 步骤展示「校验 ISO → 分发到目标机（iso.pull）→ 创建安装 VM」；
- 兼容入口：折叠「目标机已有 ISO」，仍可用 `iso_name`（v2 路径）；
- 上传/分发/安装/取消错误提示与 409 一致。

## B.6 验收

1. `cargo test`：ISO 名称/状态机/权限/seed 解析/分发编排单测；agent iso.pull 单测；
2. 真机：把 Win10 LTSC ISO 放入 `ISO_SEED_DIR` → 启动扫描登记 preset → 教师端库中可见；
   构建会话 `iso_id` → preparing 显示分发（目标机已有则 skipped）→ installing（VNC Windows 安装界面）；
3. 上传一个较小 ISO（如 virtio-win 754MB）→ ready；用其创建会话 → agent 通过 presigned URL 下载到 ISO 目录（pulled=true）→ VM 创建；取消回收；
4. 教师端 type-check/lint/test/build 通过。

## B.7 服务端实现备注（v3，2026-11-14）

- **迁移 0041** `0041_install_isos.sql`：`install_isos`（filename、display_name、
  description、size_bytes、sha256、status uploading|ready|failed、storage_key、
  is_preset、uploader_user_id、时间戳；`lower(filename)` 唯一索引）+
  `image_build_sessions.iso_id`（`ON DELETE SET NULL`）/`iso_pull_request_id` 与索引；
  全部幂等（真实库事务回滚验证）。
- **S3 能力**（`qe-storage`）：新增 `presign_get_with_ttl(key, ttl)`（TTL 60s..86400s，
  ISO 分发用 7200s/2h）与 `put_file(key, path, content_type, timeout)`（`ReaderStream`
  流式上传 + Content-Length，默认客户端 60s 超时被单请求 timeout 覆盖）。
- **上传接口**：`PUT /admin/isos/{id}/content` 注册在 **独立长上传路由**（`api/mod.rs`
  `iso_upload_router`），不经过全局 2 MiB body 限制 / 30s 超时；上限 16 GiB、
  超时 2h。handler 边收边写 `<DATA_DIR>/iso-uploads/<id>.part` 并计算 SHA-256，
  完成后流式上传 S3 并置 ready；任一步失败置 failed + 清理临时文件（超限 413）。
- **seed 扫描**：`ISO_SEED_DIR`（默认 `<DATA_DIR>/iso-seeds`，`DATA_DIR` 默认
  `/var/lib/qe-control`）；`sha256_file` 1 MiB 流式读取（4.7GB 不占内存），
  `put_file` 流式上传；同 filename 同 size+sha → skipped，同名不同内容/状态 → failed
  项（不覆盖已有记录），否则登记 preset ready。启动时后台跑一次（失败只 warn），
  `POST /admin/isos/seed/scan` 管理员手动触发。
- **构建会话**：`iso_id` 与 `iso_name` 二选一（同时给/都不给 → 400）；`iso_id` 需
  ISO `ready` 且可见（preset 所有人、非 preset uploader/admin）。preparing 首步在
  POST 时生成 presigned GET（TTL 2h）并 `vm.command iso.pull`
  `{name,url,sha256,size_bytes}`，记录 `iso_pull_request_id`；GET 轮询 ok → `create`，
  失败 → failed，pending 超过 30 分钟 → failed。steps（iso_id）为
  `iso_check → iso_distribute → create_vm → install → eject_iso → shutdown → upload`
  （iso_name 兼容路径仍 6 步）；回复新增 `iso_id`。
- **删除保护**：仅拦非终态会话引用（`iso_id` + state 非终态）；已结束会话经
  `ON DELETE SET NULL` 保留历史但不阻塞删除。S3 对象删除失败只 warn（记录仍删）。
- **vm.command**：白名单新增 `iso.pull`；服务端校验 `name`（安全文件名）、`url`
  （http/https + host）、`sha256`（可选 64 hex，自动小写）、`size_bytes`（可选 >0），
  非法/未知字段 400。
- **本期不做**：分片/断点续传上传（单次 HTTP PUT）、ISO 内容去重（同内容不同文件名
  各自一份）、上传进度服务端记录（前端按 HTTP 上传进度展示）。

## B.8 集成备注（2026-11-14 真机联调）

- **qe-api 启动配置**：ISO 分发依赖 presigned GET 的 host，`S3_ENDPOINT` 必须是远程 agent 可访问的控制节点 LAN 地址
  （本环境 `http://192.168.8.231:9000`；`.env` 已同步），否则 agent 下载 `127.0.0.1` 失败；
  `DATA_DIR=/var/lib/qe-control`、`ISO_SEED_DIR=/var/lib/qe-control/iso-seeds` 需进程可写。
- **系统预设 ISO**：把 ISO 放入 `ISO_SEED_DIR` → qe-api 启动后台扫描或 `POST /admin/isos/seed/scan`；
  首次 4.7GB 文件流式 SHA-256 + S3 上传约 3-4 分钟（不阻塞 HTTP），登记为 `is_preset=true` 后所有教师可见。
- **上传**：`POST /admin/isos` → `PUT /admin/isos/{id}/content`（单次原始 body，16GiB/2h 级长上传路由，
  免全局 2MiB/30s 限制）；服务端流式落临时文件 → SHA-256 → S3 → ready。实测 754MB 约 43s（约 18MB/s）。
- **分发**：iso_id 会话 preparing 先 `iso_check` 生成 2h presigned GET → `iso_distribute` 下发
  `vm.command iso.pull`；目标机已有同 SHA 文件时 agent 秒回 skipped；754MB 上传 ISO 首次分发约 70s；
  之后才创建安装 VM。
- **修复**：`image_build_sessions` INSERT 在新增 `iso_id` 列后绑定顺序错位（vcpus 收到 text）——
  session 创建 500；已修正列/占位符/绑定顺序（与 0040 的 base_image_id 同类问题）。
- **清理**：取消/完成会话回收构建 VM；测试上传的非预设 ISO 可由上传者/admin DELETE（有非终态会话引用 409）。

---

# 附录 C：ISO 分片续传 / 共享审批 / 镜像导出优化（v4，2026-11-14）

## C.1 分片可续传 ISO 上传（替换单次 PUT）

### DDL（migration 0042）

```sql
ALTER TABLE install_isos
  ADD COLUMN upload_chunk_size bigint,
  ADD COLUMN upload_total_chunks int,
  ADD COLUMN upload_updated_at timestamptz;

CREATE TABLE install_iso_chunks (
  iso_id uuid NOT NULL REFERENCES install_isos(id) ON DELETE CASCADE,
  chunk_index int NOT NULL CHECK (chunk_index >= 0),
  size_bytes bigint NOT NULL CHECK (size_bytes > 0),
  sha256 text NOT NULL,
  received_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (iso_id, chunk_index)
);
```

- 默认 chunk_size = 64 MiB（服务端可配 `ISO_UPLOAD_CHUNK_MIB`，8..256），总上限 16 GiB；
- 每个 chunk 允许 1..chunk_size 字节（最后一块可小）；服务端按请求体实际长度校验并记录 SHA-256；
- 分片文件落 `DATA_DIR/iso-uploads/<iso_id>/<index>.part`，finalize 时按序流式拼接→整体 SHA-256→S3→ready；
- 并发写不同 part 文件，天然支持多连接并发上传；重复 chunk 幂等覆盖；
- 目标：4.7GB ISO 可断点续传、可并发、可暂停/取消；失败只重传失败分片。

### API

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/admin/isos` | body 增 `size_bytes`（必填，>0、≤16GiB）与可选 `chunk_size_mib`；返回 `{id, upload:{chunk_size,total_chunks,received:[]}}`；同名同 size 且属于当前用户且 uploading → 幂等返回原记录（用于续传）；同名 ready → 409 |
| GET | `/admin/isos/{id}/upload` | 上传状态：`{status,size_bytes,chunk_size,total_chunks,received:[indices],received_bytes,sha256?}`（owner/admin） |
| PUT | `/admin/isos/{id}/chunks/{index}` | 原始 body（1..chunk_size）；校验 index、长度、SHA-256；落 part 文件 + 记 chunk 行；返回 `{received_count,received_bytes}` |
| POST | `/admin/isos/{id}/complete` | body `{sha256?}`；校验所有 chunk 齐全、大小和=size_bytes；拼接+校验（若传 sha 必须匹配）→ S3 `isos/<id>/<filename>` → ready；缺片 409 并返回缺失索引 |
| DELETE | `/admin/isos/{id}/upload` | 中止上传：删除 part 目录/chunk 行，记录 failed（或删除，owner/admin 可选）；释放同名占用 |
| PUT | `/admin/isos/{id}/content` | **保留 v3 单次 PUT 兼容**（≤16GiB，最终仍可完成），但教师端改用分片 |

- 可见性/权限沿用：owner/admin 可操作；`preset` 不允许上传/删除；
- 失败/超时上传保留 part 与已收 chunk，供续传；`DELETE /admin/isos/{id}` 仍可彻底删除。

## C.2 ISO 共享与审批

### DDL（同 0042）

```sql
ALTER TABLE install_isos
  ADD COLUMN visibility text NOT NULL DEFAULT 'private'
    CHECK (visibility IN ('private','shared')),
  ADD COLUMN share_status text NOT NULL DEFAULT 'none'
    CHECK (share_status IN ('none','pending','approved','rejected')),
  ADD COLUMN share_requested_by uuid REFERENCES users(id),
  ADD COLUMN share_reviewed_by uuid REFERENCES users(id),
  ADD COLUMN share_reviewed_at timestamptz,
  ADD COLUMN share_note text;
```

- 可见/可用：`is_preset` → 所有人；owner/admin → 自己/全部；`visibility=shared AND share_status=approved` → 所有 `image:read` 教师；
- 共享需管理员审批：owner 发起申请 → pending；admin approve → `visibility=shared, share_status=approved`；reject → rejected（可再次申请）；
- 审批人与时间/备注留痕；撤销共享 → private/none。

### API

| 方法 | 路径 | 权限 | 说明 |
|---|---|---|---|
| POST | `/admin/isos/{id}/share-request` | owner/admin/image:manage | 申请共享（幂等：pending 返回现有状态） |
| POST | `/admin/isos/{id}/share-cancel` | owner/admin | 取消 pending |
| GET | `/admin/isos/share-requests?status=pending` | admin | 待审批列表（含 filename/size/uploader/申请时间） |
| POST | `/admin/isos/{id}/share-review` | admin | `{approve: bool, note?}` → approved/shared 或 rejected |
| POST | `/admin/isos/{id}/unshare` | owner/admin | 撤销共享（private/none） |

- 列表响应增加 `visibility/share_status/is_shared`；教师端可看到共享徽标与申请入口；
- 管理台新增「ISO 库」页：全部 ISO、待审批（批准/拒绝）、预设扫描、删除/共享状态。

## C.3 Windows 镜像导出优化（image.make）

- **并发分片上传**：agent `image_make` 按 `IMAGE_MAKE_CONCURRENCY`（默认 8，1..16）并发 PUT 分片；
  `IMAGE_MAKE_CHUNK_MIB`（默认 64，8..256）分片大小；失败分片重试（默认 3 次）；
- **压缩导出**：`IMAGE_MAKE_COMPRESS=1`（默认开）时 `qemu-img convert -c`，Windows 10.5GB 预计压到 ~6-7GB；
  （qcow2 压缩镜像可直接作为 libvirt 卷/backing，导入逻辑无需改）
- **零分片跳过**：全零 chunk 标记为 skipped（manifest 已支持 skipped_chunks），减少无效上传；
- **服务端 pending 超时**：`IMAGE_MAKE_PENDING_TIMEOUT_SECS` 默认 1800s（原 600s），`image.make` 命令/结果逻辑不变；
- **进度**：agent 每 N 个分片记录日志（chunk/total/字节/耗时/吞吐）；服务端仍按 request_id 轮询；
- **验收**：用已装好的 Windows 10 LTSC（10.5GB）导出，记录导出/上传耗时；服务端 verify/publish 通过；
  并验证 Windows 底包用 `disk_bus=sata` 创建构建 VM 能进桌面。

## C.4 服务端实现备注（v4，2026-11-14）

- **迁移 0042** `0042_iso_chunks_sharing.sql`：`install_isos` 增
  `upload_chunk_size/upload_total_chunks/upload_updated_at` 与
  `visibility/share_status/share_requested_by/share_reviewed_by/share_reviewed_at/share_note`；
  新表 `install_iso_chunks(iso_id, chunk_index, size_bytes, sha256, received_at, PK(iso_id,chunk_index))`；
  `visibility` / `share_status` 命名 CHECK + pending 审批索引；幂等（真实库事务内双次 apply + SQL 冒烟后 ROLLBACK）。
- **分片计划**：`POST /admin/isos` 必填 `size_bytes`（>0、≤16 GiB）、可选
  `chunk_size_mib`（8..256，缺省 `ISO_UPLOAD_CHUNK_MIB`，默认 64）；返回
  `{id, upload_url, upload:{chunk_size(字节), chunk_size_mib, total_chunks, received[]}}`。
  同名同 owner + 同 size + `uploading` → **200 幂等返回原记录**用于续传；同名 `ready` → 409。
- **分片文件**：`<DATA_DIR>/iso-uploads/<iso_id>/<index>.part`（先写 `.part.tmp` 再原子 rename）；
  `PUT /admin/isos/{id}/chunks/{index}` 原始 body，长度必须等于
  `min(chunk_size, size - index*chunk_size)`（非末片必须满片）；可选 `x-chunk-sha256` 头或
  `?sha256=` 查询参数校验；重复 index 幂等覆盖；返回 `{received_count, received_bytes}`。
  允许 `uploading|failed`（失败后保留分片可续传）。
- **complete**：`POST /admin/isos/{id}/complete` body `{sha256?}`（`{}` 亦可）；校验片数齐全，
  缺片 → 409 `{"error":{"code":"missing_chunks","message":...,"missing":[索引]}}`；
  各分片大小和必须等于 `size_bytes`；按 index 顺序 1 MiB 缓冲流式拼接 + 整体 SHA-256（可选
  sha 必须匹配，否则 400）→ `put_file` S3 → ready → 清理分片行/目录。任一步失败置 failed
  **保留分片**供续传（临时合并文件会清理）。
- **中止**：`DELETE /admin/isos/{id}/upload` 仅 `uploading|failed`，owner/admin，preset 拒绝；
  删除分片行 + 目录 + 记录本身（释放同名可立即重新上传）。
- **v3 兼容**：`PUT /admin/isos/{id}/content` 保留；若 POST 声明了 size，单次 PUT 结束会校验
  实际大小一致；成功后清理可能残留的分片数据。
- **共享/审批**：可见性 = preset 全员 / owner / admin / `visibility=shared AND share_status=approved`；
  owner 或 admin 可 `share-request`（pending 幂等，preset/非 ready 拒绝）→ 管理员
  `share-review {approve,note?}`（仅 pending；approve→shared+approved，reject→rejected）→
  `share-cancel`/`unshare` 回到 none/private；`GET /admin/isos/share-requests?status=pending`
  仅 admin（status 支持 pending/approved/rejected/none/all）。审批列表的「申请时间」用
  `updated_at`（DDL 未单列 requested_at）。
- **preset 保护**：预设 ISO 不可删除、不可上传/中止、不可申请共享；仅管理员可 seed 扫描。
- **响应**：列表/审批响应增加 `visibility/share_status/is_shared/share_note/
  share_requested_by/share_reviewed_by/share_reviewed_at`（不暴露 `storage_key`）。
- **长请求路由**：`PUT /chunks/{index}`、`POST /complete` 与 v3 `/content` 同属独立长上传路由
  （免全局 2 MiB body / 30s 超时；body 上限 16 GiB、超时 2h）。
- **本期不做**：分片并发上限/限速、跨进程分片锁（写不同 index 天然并发安全，同 index 幂等覆盖）、
  断点元数据压缩/清理 worker（中断记录由用户 DELETE /upload 释放）。
