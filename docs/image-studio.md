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
