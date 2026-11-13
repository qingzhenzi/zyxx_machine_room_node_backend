# 镜像制作生命周期与制作工单契约（MVP v1）

> 2026-11-13。配套设计：[image-lifecycle-multiteacher.md](image-lifecycle-multiteacher.md)。
> 本文件是实现契约：room_node（服务端/device-pki）、room_agent（agent/教师端）共同遵守。

## 1. 分层就绪模型（回答「上传后才就绪吗」）

| `image_artifacts.status` | 含义 | API `readiness` | 能否分配/下发 | UI 文案 |
|---|---|---|---|---|
| `draft` | 制作工单已创建，机器尚未完成导出/上传 | `building` | 否 | 制作中 |
| `uploaded` | 仅元数据（旧接口创建） | `uploaded` | 否 | 已上传（缺数据） |
| `manifest_ready` | 导出+分片上传+register 完成，数据完整 | `uploaded` | 否 | 已上传，待校验 |
| `verified` | 逐片 sha256/manifest 等校验通过 | `verified` | 否 | 已校验，待发布 |
| `published` | 已发布到通道（current） | `ready` | **是** | 可用 |
| `deprecated` / `revoked` | 下架/吊销 | `unavailable` | 否 | 已下架/已吊销 |

- **「就绪」= `published`**：agent `image.pull` / 房间分配只接受 published（现有行为）；
- 「上传完成」= `manifest_ready`（数据完整但未校验），不是可用；
- 制作链路：`draft →（image.make/register）→ manifest_ready →（verify）→ verified →（publish）→ published`；
- 前端必须按上表显示，不再把 manifest_ready 说成“就绪”。

## 2. 所有权与命名空间（MVP）

- 迁移 0036：`image_namespaces(id, kind, owner_user_id, slug UNIQUE, display_name, created_at)`；
  - 内置公共命名空间（slug=`public`，kind=public）；
  - 每个已有用户建个人命名空间（slug=规范化 username，冲突时加 `-2`）；
- `image_artifacts` 增加：`namespace_id`、`owner_user_id`、`visibility`（默认 `public`，旧数据回填 public）、
  `source_kind`、`source_room_id`、`source_machine_serial`、`source_vm_name`、`description`、`deleted_at`；
- 唯一约束：删除全局 `UNIQUE(name,version,platform)`，改为
  `UNIQUE (namespace_id, name, version, platform)`；
- 旧数据：`owner_user_id=created_by`（NULL → admin），`namespace_id` = owner 个人命名空间或 public；
- 展示名：`@<namespace.slug>/<name>:<version>`（public 可省略前缀）。

## 3. 归属校验（MVP）

- `GET /admin/images?scope=mine|public|all`（`all` 仅 admin/image:manage:all）：
  - teacher 默认 `mine + public`（后续加 shared）；
  - 响应新增 `owner_user_id, owner_name, namespace_slug, visibility, readiness`；
- `GET /admin/images/{id}`：仅 owner / admin / public 可读；教师看不到别人的私有镜像；
- 变更类接口 `verify/publish/deprecate/delete/assign`：**owner 或 admin** 才能操作；
  旧数据 owner=admin；非 owner 返回 403「无权操作他人的镜像」；
- `image:manage` 语义保留（teacher 拥有），叠加 owner 校验，不新增权限码即可上线；
- 分配（assign）：owner 或 admin；且镜像必须 `published`（现有校验保留）。

## 4. 制作工单（身份桥，关键）

### 4.1 下发 image.make（qe-api）

```text
POST /admin/machines/{serial}/vm/commands {op:"image.make", args:{vm_name,name,version,platform,base_volume?,description?}}
```

服务端在入队前（同一事务/补偿）：

1. 校验 `vm:control + image:manage`；解析 actor 的个人命名空间；
2. 预检同名：`(namespace_id,name,version,platform)` 已存在且状态非 deprecated/revoked → 409 `image_name_taken`；
3. `INSERT image_artifacts(status='draft', name, version, platform, kind='vm_disk', format='qcow2',
   owner_user_id=actor, namespace_id=personal, source_kind='vm_export',
   source_room_id=<machine.room_id>, source_machine_serial=serial, source_vm_name=vm_name,
   description)`；
4. `INSERT image_build_requests(id, image_id, machine_serial, room_id, state='pending',
   expires_at=now()+30min, name/version/platform 快照, created_by=actor)`；
5. 把 `build_request_id` **由服务端注入** 下行 args（客户端传该字段 → 400）；
6. 入队 `vm.command`；失败时回滚：删除 build_request + draft artifact。

`image.make` 命令结果新增字段：`image_id`（draft id）、`build_request_id`、`manifest_digest`。

### 4.2 register（device-pki）

`POST /machine/images/register` 请求体新增可选 `build_request_id`（UUID）。

- **有 build_request_id**：
  1. `resolve_identity` → serial/room；
  2. `SELECT ... FROM image_build_requests FOR UPDATE`，校验
     `state='pending' AND expires_at>now() AND machine_serial=$serial AND room_id 匹配`
     且 name/version/platform 与工单一致；
  3. 校验分片对象存在/大小（现有逻辑）；
  4. **更新**工单对应的 draft artifact（`status='manifest_ready', manifest_digest, total_size,
     format/kind 以工单为准`），不新建 artifact、不覆盖 owner/namespace/visibility；
  5. `UPDATE image_build_requests SET state='consumed', consumed_at=now()`；
  6. 返回 `{image_id: <draft id>, manifest_digest, status:"manifest_ready"}`；
  7. 失败：`state='failed'`，draft 保留（可重试 verify/重新 register，不产生同名孤儿）；
- **无 build_request_id**：保持旧行为（新建 artifact、`created_by=NULL`），
  兼容旧 agent/旧前端；通过环境变量 `IMAGE_BUILD_STRICT=1` 可关闭旧路径。

### 4.3 agent（room_agent）

- `image.make` args 解析可选 `build_request_id`（vm_control `ImageMakeArgs`）；
- `image_make.rs` 构造 `RegisterImageRequest` 时带上；
- `qe-device-pki-client::image::RegisterImageRequest` 增加 `build_request_id: Option<String>`（serde 跳过 None）；
- CLI `qe-agent cli vm make-image` 不暴露该字段（仅下行 API 注入）。

## 5. 教师端（Tauri）MVP

- 路由：`/`（现有机器人页）、`/images`（镜像库）、`/images/:id`（只读详情）；
  header 增加两个 Tab；`/images` 数据用 `GET /admin/images?scope=mine` + `scope=public`；
- VM 行新增「制作镜像」：可见条件 `image:manage + vm:control`；VM 运行中按钮可点，
  弹「关机并继续」；向导 4 步：选 VM（前置检查）→ 镜像信息（同名预检）→ 制作进度
  （2s 轮询，最长 10 分钟，可后台）→ 校验/发布（默认 dev）；
- 镜像库：Tab「我的镜像 / 公共镜像」，列：名称:版本、平台、大小、**状态（按 §1 readiness）**、
  可见范围、创建者、更新时间、操作（查看/发布/删除）；
- 发布后提示「已发布，可分配到房间」；未发布不可分配。

## 6. 管理台（room_node/frontend）本次顺手修复

1. `api/index.ts`：verify/publish 单独设置长超时（verify 180s、publish 60s），避免 20s 误判；
2. `MachineDetailView.vue`：制作镜像默认通道改 `dev`；verify/publish 失败提供
   「去镜像库查看/只重试校验」入口，不再只能整链重来；
3. `ImagesView.vue`：增加「归属（我的/公共/全部）」筛选、owner 列、readiness 状态标签。

## 7. 验收

1. `cargo check/test`（qe-db/qe-api/qe-device-pki/qe-agent-node/qe-device-pki-client）通过；
2. 教师端 `pnpm type-check`/`build` 通过；管理台同上；
3. 真机 E2E（192.168.8.141，dev-b104）：
   - 新建一个小 VM（1GiB 空白盘，start=false）→ 关机；
   - 教师/管理员触发 image.make：控制面出现 `draft`，build_request pending；
   - register 后：同一 artifact 变 `manifest_ready`，`owner_user_id` 非空，build_request consumed；
   - verify → `verified`；publish dev → `published`，`readiness=ready`；
   - 列表 `scope=mine` 能看到；非 owner 调 verify/publish 返回 403；
   - 同名同版本再次 image.make 在下发前 409（不再白传）。
