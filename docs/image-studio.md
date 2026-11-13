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
