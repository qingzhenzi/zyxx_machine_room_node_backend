# 镜像制作与多教师镜像管理设计（v1，待拍板）

> 状态：设计稿（2026-11-13）。事实审计、后端模型、UX 三份详细报告见
> `tmp/image-mgmt-audit.md`、`tmp/design-image-management-backend.md`、
> `tmp/design-image-management-ux.md`（未提交，作为本文附录）。
> 本文是可执行结论：现状 → 目标模型 → DDL/API/页面 → 分期。

---

## 0. 一页结论

1. **「GUI 制作镜像」不是没有，而是藏得深**：入口在「机器详情 → 虚拟机行内 → 制作镜像」，
   要求 `image:manage + vm:control`、VM 必须关机；成功后自动 verify + publish。
   「镜像」页的「新建镜像」只建元数据、不传文件，所以从镜像库出发确实“做不了”。
2. **教师桌面端（Tauri）完全没有镜像能力**；Web 管理台的教师工作台 `/teacher` 有“镜像管理”快捷入口。
3. **「从零制作系统镜像」GUI 链路是断的**：`vm.command create` 白名单没有 ISO/CD-ROM/启动顺序，
   管理台“新建虚拟机”也没有 ISO；只能上机器用 CLI 装系统，教师无法在界面完成“装系统 → 关机 → 制作镜像”。
4. **多教师治理基本为零**：`created_by` 存在但 GUI 制作路径恒为 `NULL`，列表/权限/唯一约束都没用；
   任何 `image:manage` 可操作任何人的镜像；全局 `UNIQUE(name,version,platform)`；
   无可见范围/共享/审批/配额；发布会把同通道别人镜像的 `is_current` 置 false。
5. **建议 MVP**：先补“制作工单（owner 有主）+ 所有权/同名冲突 + 教师端制作向导 + 镜像库 + 发布审批”，
   再做共享/配额/通道隔离，最后做 ISO/cloud-init 模板制作。分期见 §7。

---

## 1. 现状审计（带证据）

### 1.1 管理端 Web：有制作能力，但入口/体验不好

链路：机器详情 VM 行 →「制作镜像」弹窗（名称/版本/平台/通道/底包/说明）→
`vm.command op=image.make`（agent 导出 qcow2 → 32MiB 分片 → presign/PUT → register）→
前端串行 verify → publish。

- 按钮：`room_node/frontend/src/views/MachineDetailView.vue:1157-1170`；弹窗 `:1569-1689`；
  自动 verify+publish `:723-751`；可见条件 `image:manage && vm:control`（`:625`）；
  VM 运行中禁用（agent 侧 `ensure_shut_off`，`room_agent/crates/qe-agent-node/src/vm_control.rs:609-629`）。
- 镜像页「新建镜像」只建元数据（`ImagesView.vue:129-131`、`CreateImageModal.vue:92-100`；
  `SubmitManifestModal` 是粘贴 JSON），**不能从镜像页制作**。
- 现有弹窗实测截图：

![制作镜像-现有管理台弹窗](../../tmp/ui/image-make-modal-admin.png)

体验问题（全部有代码证据）：

| 问题 | 证据 | 影响 |
|---|---|---|
| 默认发布通道 `prod`，verify 后立即 publish，无审批 | `MachineDetailView.vue:667`、`:741-751` | 教师可直发生产 |
| 无真实进度：只有 3 个粗粒度步骤，运行中文案固定“最长 10 分钟” | `:627-633`、`:695-711`、`:1634-1667` | 大镜像无反馈、无法判断卡住 |
| 自动 verify 用 axios 默认 **20s** 超时，服务端 verify 是同步逐分片下载 | `frontend/src/api/index.ts:187`、`:689-690`；`crates/api/src/api/admin/images.rs:607-687` | 大镜像前端先失败，服务端可能仍成功 → UI/服务端状态不一致 |
| 三方 600s 超时竞争（前端轮询 / 控制面 pending / agent 执行），超时后 agent 晚到结果被丢弃 | `MachineDetailView.vue:290-291,624,706-711`；`machines.rs:448-449,1595-1608`；`device-pki/src/command.rs:55-108` | 可能留下 `manifest_ready` + `created_by=NULL` 的孤儿镜像，UI 不知道 |
| 失败重试会完整重新导出上传，然后撞全局唯一 409 | `MachineDetailView.vue:1683`（按钮仍可点）、`device-pki/src/images.rs:477-495`、`0011:34` | 浪费 10 分钟且必然失败 |
| 制作过程无取消接口、无进度字段 | `migrations/0031_machine_command_results.sql` 无 progress；`transfers.progress` 与制作无关 | 教师只能等/关页面 |
| agent 允许 64GiB，device-pki 只收 32GiB | `room_agent/.../image_make.rs:31-32` vs `room_node/crates/device-pki/src/images.rs:33-38` | 32–64GiB 白传后失败 |

### 1.2 教师桌面端（room_agent/apps/teacher-client）：完全缺失

- 路由只有 `home`（`src/router/index.ts:13-24`），全目录无 `image/镜像`；
- `TeacherHome.vue` 只能登录、房间/机器列表、VM 启停/强制停止、容器启停；
- 无 VNC、无新建 VM、无镜像库/制作入口；
- 附加问题：默认账号 admin、用户名/密码明文存 localStorage（`TeacherHome.vue` 登录逻辑）、
  access token 15 分钟无 refresh —— 做镜像长任务前必须修（否则 10 分钟任务中途 401）。

### 1.3 「制作系统镜像」的前置缺口：GUI 装不了系统

- CLI 有能力：`qe-agent vm create --iso <iso> --boot cdrom`（`room_agent/apps/qe-agent/src/vm.rs`）；
- 下行 API 没有：`VM_CREATE_ARGS` 只有 `name/vcpus/memory_mib/disk_gb/base_volume/base_path/gpu/start`
  （`room_node/crates/api/src/api/admin/machines.rs:500-509`）；
- 管理台「新建虚拟机」表单同样无 ISO（`MachineDetailView.vue` VM 创建弹窗）；
- 教师端无 VNC（管理台有 noVNC：`VncConsoleModal.vue`）。

结论：要“GUI 制作系统镜像”，必须先补 **模板 VM 创建（ISO/cloud image）+ 安装期 VNC**，
或改用预置 cloud-init/模板机方案；否则只能“从别人装好的 VM 制作”。

### 1.4 多教师治理缺口

| 问题 | 证据 | 风险 |
|---|---|---|
| GUI 制作路径 `created_by=NULL` | `device-pki/src/images.rs:471,487`；`machines.rs:1367-1406` 无 actor | 镜像无主，无法审计/配额 |
| 只有权限点没有归属校验 | `images.rs:380,617,817,1083,1128` 等全部只查 `image:manage/read` | 任何人可操作任何人的镜像 |
| 全局唯一 `(name,version,platform)` | `migrations/0011_storage.sql:34` | 两位老师同名版本互斥；冲突在完整上传后才 409 |
| 无可见范围/共享 | 无字段/表；`image:read` 即全局可读 | 私有镜像泄露/误分配 |
| 无发布审批 | `images.rs:810-851` 只要求 verified + image:manage | 教师直发 prod |
| 发布 `is_current` 全局翻转 | `crates/db/src/storage.rs:794-811` | 多人 prod 互相顶掉 |
| 无配额 | 仅单对象/单镜像上限；`storage-design.md` 把配额列 P2 | 单教师占满磁盘 |
| 删除级联删分配/通道/验证 | `0011:53,64,74`；`storage.rs:1226-1247` | 误删影响已分配房间 |
| 教师无 `audit:read` | `0033_teacher_role.sql` | 镜像变更无法自查 |

---

## 2. 目标与原则

**目标**：教师在一个界面完成「准备模板 VM（装系统）→ 关机 → 制作镜像 → 校验 → 申请发布 → 共享/分配」；
管理员负责审核、配额、全量管理与归属转移；每个镜像可回答「谁的、谁能用、从哪来、谁批的」。

原则：

1. 兼容现有 `image.make/verify/publish/assign`、设备 manifest/presign 链路；
2. 所有权是核心：先解决“镜像有主 + 同名不冲突”，再谈权限码；
3. 设备侧不可信：register 必须绑定服务端预创建的「制作工单」，不能直接传 owner；
4. 分阶段可上线/可回滚；
5. 不做完整多租户：以“个人命名空间 + 公共命名空间 + 房间/区域共享”为边界，团队后续扩展。

---

## 3. 核心设计

### 3.1 两条制作路径

| 路径 | 场景 | 交互 |
|---|---|---|
| **A. 已有 VM → 镜像（MVP）** | 已有装好系统的 VM | 教师端/管理台向导：选机器 → 选 VM（关机）→ 镜像信息 → image.make → verify → 草稿/申请发布，后台任务中心跟踪 |
| **B. ISO/cloud-init → 模板系统镜像（P3）** | 从零装系统 | 新建模板 VM（挂 ISO 或 cloud image，boot=cdrom）→ 内嵌 noVNC 安装 → 关机 → 走 A |

B 的后端改动：`vm.command create` 白名单加 `iso_path|iso_url`、`boot`、`nic`
（agent `CreateArgs` 已支持，只差 API 校验/下发）；教师端加 VNC 控制台（复用现有 `vnc.open` 下行与
`VncConsoleModal` 逻辑）。

### 3.2 所有权：命名空间

- 新增 `image_namespaces(id, kind, owner_user_id, slug, display_name, created_at)`：
  - `kind=personal`：每个用户一个个人命名空间（`slug=<username>`）；
  - `kind=public`：内置公共命名空间（admin 管理）；
  - 后续可加 `kind=team/area/room`。
- `image_artifacts` 增加 `namespace_id`；唯一约束改为
  **`UNIQUE (namespace_id, name, version, platform)`**（允许不同老师同名同版本）。
- 展示限定名：`@<slug>/<name>:<version>`；公共镜像可省略命名空间显示。
- 旧数据迁移：`namespace_id` 回填到公共命名空间（NULL owner）或 owner 的个人命名空间；
  旧全局唯一约束先保留到新索引建好再删（见 §4.5/后端附录）。
- 发布到“公共”时通过 `image_public_names(name, version, platform)` 做全局唯一占用，
  避免公共库冲突（可后置）。

### 3.3 可见范围与共享

`image_artifacts.visibility`：`private | room | area | public`（默认 `private`；旧数据回填 `public`）。

- `private`：owner + admin；
- `room/area`：通过 `image_shares(image_id, scope_type, scope_id, permission)` 精确共享
  （scope_type=user/room/area；permission=read/assign）；
- `public`：有 `image:read` 均可读/可分配。

分配校验：`assign_room` 必须检查「该镜像对目标房间可见」；device-pki `authorize_image`
依旧是“分配即授权”，但 API 层不再允许把私有镜像分配到别人房间；同时校验镜像
`published`、分配未撤销、通道存在 current 记录（详见后端附录 §6.2）。

### 3.4 状态机与发布审批

现有：`uploaded → manifest_ready → verifying → verified → published → deprecated/revoked`。

新增（**不改旧 /publish 响应**）：

```
verified --(教师)--> 创建 image_publish_requests(pending)
                         ├─ approve → 复用现 publish 事务 → published
                         └─ reject  → verified + comment
```

- `image_publish_requests(id, image_id, channel, status, requested_by, reviewed_by, comment, timestamps)`；
- 镜像聚合展示状态增加 `draft`、`pending_publish`（不新增 DB 枚举也可，用 request 状态聚合）；
- 权限：新增 `image:publish:submit`（teacher）、`image:publish:approve`（admin）、
  `image:build`（制作）、`image:share`、`image:assign`、`image:quota:manage`、
  `image:manage:all`（admin 全量）；教师保留 `image:manage` 但语义 = 「管理自己的镜像」。

### 3.5 身份桥：draft + 制作工单（关键）

设备 register 只有设备证书身份，没有教师身份。不能给 register 加 owner 参数（可伪造），
必须在**下发 image.make 前**在控制面落身份：

```text
教师 UI → POST /admin/machines/{serial}/vm/commands {op:image.make, args:{vm_name,name,version,...}}
  qe-api（vm:control + image:build）:
    1. 解析 owner/个人 namespace；检查数量配额
    2. INSERT image_artifacts(status='draft', owner_user_id, namespace_id,
         source_kind='vm_export', source_machine_serial, source_room_id, source_vm_name)
    3. INSERT image_build_requests(id, image_id, machine_serial, state='pending', expires_at=now()+15min,
         name/version/platform 快照)
    4. 由服务端把 build_request_id 注入 command.args（客户端不可传）
    5. 入队 vm.command
  agent → 导出/分片上传 → register(build_request_id)
  device-pki:
    校验 build_request（pending、未过期、serial 匹配、name/version/platform 一致）→
    UPDATE artifact SET manifest_digest/total_size/status='manifest_ready'（仅 draft）→
    build_request state='consumed' → image_events(device)
  失败：build_request='failed'，保留 draft 供重试（不产生同名孤儿）；过期由定时任务清理。
```

好处：GUI 制作的镜像天然有主；同名/重试在 draft 阶段就能 409；不再有 `created_by=NULL` 孤儿。

### 3.6 配额

`image_quotas(scope_type user/role/global, scope_id, max_count, max_bytes)`；
生效顺序 user → role → global；检查点：

- 下发 image.make 前（数量、预占），register 时（字节，事务 + advisory lock）；
- 删除/下架/过期草稿释放；
- 统计口径 `SUM(total_size) WHERE owner/namespace AND status NOT IN ('deprecated','revoked')`。

### 3.7 权限矩阵（目标态）

| 操作 | admin | owner 教师 | 其他教师（有共享） | 其他 |
|---|---|---|---|---|
| 查看私有 | ✅ | ✅ | ❌ | ❌ |
| 查看 public/共享 | ✅ | ✅ | ✅ | ✅ image:read |
| 制作（image.make） | ✅ | ✅ | ❌ | ❌ |
| verify 自己的 | ✅ | ✅ | ❌ | ❌ |
| 提交发布申请 | ✅ | ✅ | ❌ | ❌ |
| 审批 test/prod | ✅ | ❌ | ❌ | ❌ |
| 直接发布 dev | ✅ | ✅ | ❌ | ❌ |
| deprecate/delete 自己的 | ✅ | ✅ | ❌ | ❌ |
| 分配到房间 | ✅ | ✅ | ✅（可见即可） | ❌ |
| 共享/改可见范围 | ✅ | ✅ 自己的 | ❌ | ❌ |
| 配额/归属转移/全量 | ✅ | ❌ | ❌ | ❌ |

旧角色不重新授权也能继续工作：教师已有 `image:manage`，越权操作由 owner/visibility 校验拦 403；
新增权限码按迁移播种。

### 3.8 审计

新增 `image_events(id, image_id, actor_kind user/device/system, actor_user_id, actor_serial,
event, payload, created_at)`，覆盖：`build.requested`、`register`、`verify`、`publish.requested`、
`publish.approved/rejected`、`publish`、`deprecate`、`delete`、`share`、`assign`。
教师端镜像详情可按时间线展示（教师无 `audit:read`，走专用只读投影）。

---

## 4. 数据模型（迁移草案，详细 DDL 见后端附录）

```sql
-- 0036_image_ownership.sql（M0/M1）
CREATE TABLE IF NOT EXISTS image_namespaces (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  kind TEXT NOT NULL CHECK (kind IN ('personal','public','team','area','room')),
  owner_user_id UUID REFERENCES users(id) ON DELETE CASCADE,
  slug TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- 内置公共命名空间 + 为现有用户建个人命名空间
ALTER TABLE image_artifacts
  ADD COLUMN namespace_id UUID REFERENCES image_namespaces(id),
  ADD COLUMN owner_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN visibility TEXT NOT NULL DEFAULT 'public',
  ADD COLUMN source_kind TEXT,
  ADD COLUMN source_room_id UUID REFERENCES rooms(id) ON DELETE SET NULL,
  ADD COLUMN source_machine_serial TEXT,
  ADD COLUMN source_vm_name TEXT,
  ADD COLUMN description TEXT,
  ADD COLUMN deleted_at TIMESTAMPTZ;
-- 回填：namespace_id/owner_user_id；旧数据 visibility='public'
-- 唯一约束替换（低峰）：DROP 旧 UNIQUE(name,version,platform)
CREATE UNIQUE INDEX image_artifacts_ns_name_version_platform
  ON image_artifacts (namespace_id, name, version, platform);

-- 0037_image_approval_share_quota.sql（M3/M4）
CREATE TABLE image_publish_requests (...);
CREATE TABLE image_shares (...);
CREATE TABLE image_quotas (...);
ALTER TABLE image_channels ADD COLUMN namespace_id UUID;
-- is_current 改为按 (namespace_id, channel) 唯一

-- 0038_image_build_events.sql（M2）
CREATE TABLE image_build_requests (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  image_id UUID NOT NULL REFERENCES image_artifacts(id) ON DELETE CASCADE,
  machine_serial TEXT NOT NULL,
  room_id UUID,
  name TEXT NOT NULL, version TEXT NOT NULL, platform TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'pending',  -- pending|consumed|failed|expired
  expires_at TIMESTAMPTZ NOT NULL,
  created_by UUID REFERENCES users(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  consumed_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX image_build_requests_pending_uniq
  ON image_build_requests (machine_serial, name, version, platform)
  WHERE state = 'pending';
CREATE TABLE image_events (...);
```

> 旧唯一约束替换步骤、回滚/灰度（`IMAGE_OWNER_ENFORCE`、`IMAGE_LEGACY_CLAIM`）见后端附录 §4.5、§7。

---

## 5. API 草案

### 5.1 新增/扩展

| 方法/路径 | 权限 | 说明 |
|---|---|---|
| `GET /admin/images?scope=mine|shared|public&owner=&visibility=&q=` | image:read | 列表按可见范围裁剪；返回 owner/namespace/visibility/source/size |
| `GET /admin/images/summary` | image:read | 各 Tab 计数 + 配额（避免多次全量） |
| `POST /admin/images/{id}/publish-requests` | image:publish:submit + owner | `{channel, comment}` → 202 |
| `POST /admin/images/publish-requests/{id}/approve|reject` | image:publish:approve | 通过后复用 publish 事务 |
| `GET /admin/images/publish-requests?scope=mine|pending` | 对应权限 | 待审核/我的申请 |
| `POST/DELETE /admin/images/{id}/shares` | image:share + owner | 共享到 user/room/area |
| `PATCH /admin/images/{id}` | image:manage + owner/admin | 改 visibility/description（不可改名） |
| `GET /admin/images/quotas/me`；`PUT /admin/image-quotas` | image:read / image:quota:manage | 配额 |
| `POST /admin/images/{id}/transfer` | user:manage | 归属转移 |
| `POST /admin/machines/{serial}/vm/commands {op:image.make}` | vm:control + image:build | 服务端预创建 draft + build_request，注入 id |
| `GET /admin/machine-commands/{rid}` | vm:control | 结果含 `image_id/build_request_id` |

### 5.2 兼容策略

- 现有 `POST /images/{id}/publish` 保留：admin/审批人直接发布；teacher 调它时若
  `channel != dev` → 返回 403 并提示走 `publish-requests`（灰度期可用开关允许旧行为）；
- 现有 `verify/deprecate/delete/assign` 增加 owner/visibility 校验，其余行为不变；
- `image.make` 的 args 白名单**不接受**客户端传 `build_request_id`（服务端注入）；
- 旧客户端（管理台旧构建）仍能制作：走旧 register 分支，`created_by=NULL` 由
  `IMAGE_LEGACY_CLAIM=true` 过渡认领，M2 后关闭。

---

## 6. 前端设计

### 6.1 教师桌面端（Tauri）：壳层 + 两个页面

现状单页 `TeacherHome.vue`，建议抽 `TeacherLayout.vue`：

```text
┌ QEStudio 教师工作台   [机房控制] [镜像库 ③]   [制作任务 ●1] [B104 ▾] [admin ▾] ┐
│ 页面内容                                                                      │
└───────────────────────────────────────────────────────────────────────────────┘
```

- 路由：`/`（机房控制）、`/images`（镜像库）、`/images/:id`（详情）；
  制作向导用右侧 `AppDrawer`（600px），不占独立路由，保留“当前机器/VM”上下文；
- `ImageTaskCenter.vue`（440px 抽屉）：进行中/待处理/已完成，显示阶段、已用时、
  错误摘要、`request_id`；关闭 App 后从 localStorage 恢复；**不提供“取消”**，只有“后台继续/放弃跟踪”；
- 登录安全顺手修：不再把明文密码写 localStorage，改用 refresh token/系统钥匙串；
  401 先 refresh，失败把任务置 `paused_auth` 而不是“制作失败”。

### 6.2 制作向导（4 步）

1. **选择源 VM**：机器在线、VM 关机、配额检查；运行中的 VM 按钮可点，弹“关机并继续”
   （shutdown → 轮询最多 60s → 可强制停止，二次确认）；
2. **镜像信息**：名称/版本/平台/可见范围/说明；**同名同命名空间预检**（第 2 步内联 409）；
3. **制作进度**：真实阶段（检查 → 机器制作中 → 校验 → 发布），机器制作中**用不确定进度条**
   （agent 不提供分片进度），显示已用时 / 最长 10 分钟；可后台；
4. **校验结果与发布**：校验通过 → 保存草稿 / 发布到我的房间 / 提交公共审核（备注）；
   校验失败 → 重试校验 / 删除这次制作 / 返回镜像库（不再整链重试）。

### 6.3 教师端镜像库

Tab：**我的镜像 / 共享给我 / 公共镜像 / 待审核**；配额条；搜索/平台/状态/可见范围筛选；
列：镜像(`name:version` + id)、平台、大小、状态、可见范围、创建者、来源房间、更新时间、操作。
行操作按状态：草稿→发布/提交审核；manifest_ready→继续校验；审核中→查看/撤回；
已驳回→重新提交；已发布→分配到我的房间/分享/下架。

### 6.4 管理端 Web

- `ImagesView.vue`：增加「我的/公共/共享」与 owner/visibility 列、配额条；
- `ImageDetailView.vue`：owner/来源/可见范围/审批记录/共享管理/提交发布/审批按钮；
- 新增 `ImageApprovalsView.vue`（路由权限 `image:publish:approve`）：待审核列表、通过/驳回/备注；
- `MachineDetailView.vue` 制作弹窗：默认通道改 dev、加可见范围、同名预检、制作完成不直接 publish
  而是“保存草稿/提交发布”、关闭弹窗不丢任务（任务中心）。

### 6.5 线框（教师端）

```text
镜像库
┌ 镜像库                                        [刷新] [从 VM 制作] ┐
│ [我的镜像 6] [共享给我 3] [公共镜像 9] [待审核 1]                 │
│ 配额 12.4 / 50 GiB ███████░░░░░ 25%                               │
│ 搜索[______] 平台[▾] 状态[▾] 可见[▾]                              │
│ 镜像            平台        大小 状态   可见  创建者 来源 更新 操作│
│ lab-base:1.2.0  linux/amd64 2.1G 已发布 公共  李老师 B104  2h [分配]│
│ lab-win:1.0.0   windows/... 4.8G 草稿   私有  李老师 B104  1d [发布]│
└───────────────────────────────────────────────────────────────────┘

制作向导（Drawer）
① 选择源 VM：✓权限 ✓机器在线 ✓已关机 !配额 12.4/50GiB
② 镜像信息：名称/版本/平台/可见范围/说明（同名预检）
③ 制作进度：✓检查 → ●机器制作中（不确定进度条，已用03:21/10:00）→ ○校验 → ○发布
④ 完成：✓校验通过（12项）→ [保存草稿] [发布到我的房间] [提交公共审核]
```

---

## 7. 分期计划（建议）

> 1 后端 + 0.5 前端估算，仅排序不承诺；每期可独立上线/回滚。

| 期 | 内容 | 关键交付 | 文件 |
|---|---|---|---|
| **M0**（0.5–1d） | 迁移 + 只读字段 | namespace/owner/visibility 列与权限码；旧链路不改行为 | `migrations/0036...`、domain/db row mapping、OpenAPI |
| **M1**（2–3d） | 所有权 + 同名冲突 + 我的镜像 | 唯一约束按 namespace；所有变更 handler owner 校验；列表 scope=mine/shared/public | `crates/db/src/storage.rs`、`crates/api/src/api/admin/images.rs`、`frontend/views/ImagesView.vue`、`ImageDetailView.vue` |
| **M2**（2–3d） | 制作工单（GUI 制镜像有主） | draft + build_request + register 绑定；`created_by=NULL` 消失；配额数量预检 | `migrations/0038...`、`machines.rs`、`db/storage.rs`、`device-pki/src/images.rs`、agent `image_make.rs`/vm_control、`MachineDetailView.vue` |
| **M3**（2–3d） | 发布审批 + 公共名 | 提交/审批/驳回/撤回；管理台审核队列；制完不自动 publish | `migrations/0037...`、`images.rs`、`admin/mod.rs`、`ImageApprovalsView.vue`、教师端镜像库 |
| **M4**（2–3d） | 配额 + 共享 + 通道隔离 + 下发校验 | `image_quotas`/`image_shares`；`image_channels.namespace_id`；`authorize_image`/`assign_room` 可见性校验 | `device-pki/src/resource.rs`、`db/storage.rs`、`ImageQuotasView.vue`、共享面板 |
| **P3**（后续） | ISO/cloud-init 模板制作 + fork/派生 + 团队/区域命名空间 + 软删除/审计页 | 新建模板 VM（ISO）+ noVNC 安装；复用 A 流程 | `vm.command create` 白名单、agent `CreateArgs`、教师端 VNC、管理台模板页 |

**MVP = M0+M1+M2+M3**：教师能制作自己的镜像、镜像有主、同名不冲突、发布可审批。
若只能做一件事：**M1+M2**（否则制出来的镜像无主/可被越权操作）。

**先顺手修的现有 bug（可与 M0 并行，风险低）**：
1. 前端 verify/publish 请求单独加长超时（如 120s/60s），避免大镜像 20s 被误判失败；
2. 控制面 pending 超时与 agent 执行超时统一口径（≥900s 或分阶段状态），避免孤儿镜像；
3. 失败重试先 `GET /images?name=` 查是否已 register，存在则只重试 verify/publish；
4. agent 64GiB 与 device-pki 32GiB 上限对齐（统一 32GiB 或放宽 device-pki）；
5. 制作弹窗通道默认 `dev`，prod 仅审批后使用；
6. 教师桌面端登录不再存明文密码 + 加 refresh。

---

## 8. 风险

| 风险 | 说明 | 缓解 |
|---|---|---|
| 唯一约束迁移 | 老数据回填、并发写入 | 低峰停写/事务；先建新索引再删旧约束；保留回滚 SQL |
| 权限收紧破坏旧流程 | 教师现有 image:manage 全量可用 | 灰度开关 `IMAGE_OWNER_ENFORCE`，先日志后 403 |
| 身份桥改造跨两端 | API/device-pki/agent 都要透传 build_request_id | 旧无单分支保留开关；先做兼容再强制 |
| 长任务状态一致性 | 600s 三方超时 + 20s verify 超时 | 上述 bug 修复；UI 超时后“先查再重试” |
| 配额并发 | 两个请求同时过检查 | 事务 + advisory lock；register 二次校验 |
| 设备侧孤儿对象 | register 失败 S3 残留 | build request 过期清理 + 后续 S3 GC |
| 教师读模型越权 | 教师工作台取全局房间列表 | 本次不解决，另立“教师 scope 读模型”任务 |
| 教师端 VNC 安全 | P3 安装期内嵌 VNC | 沿用设备证书 serial + 会话 token；只允许 owner 房间 |

---

## 9. 待拍板问题（建议默认值）

1. 教师能否自助发布 `prod`？**建议：dev 自助；test/prod 需审批。**
2. 归属按个人还是教学组？**建议：先个人命名空间 + 房间/区域共享，团队后置。**
3. 配额按人还是按区域？**建议：全局默认值 + 按用户覆盖，区域后置。**
4. “制作系统镜像”是否必须支持从 ISO 装系统？**建议：MVP 先 A（现成 VM）；P3 再做 ISO/cloud-init。**
5. MVP 范围？**建议 M1+M2 先落地（有主 + 制作工单），M3 审批紧随。**

---

## 10. 参考附录（子代理产出，未提交）

- `tmp/image-mgmt-audit.md`：现状事实审计（file:line、9 项能力结论表、风险）。
- `tmp/design-image-management-backend.md`：命名空间/工单/审批/配额完整 DDL、API 权限矩阵、
  迁移与灰度、M0–M4 文件级清单。
- `tmp/design-image-management-ux.md`：教师端壳层/任务中心/四步向导/镜像库/管理端审核队列
  的完整交互、文案、ASCII 线框与分期。
