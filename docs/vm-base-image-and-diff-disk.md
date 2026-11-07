# VM 底包 + 差分盘 + 持久化快照 设计（阶段 1.5）

> 状态：约定为本轮实现契约；「制作镜像（导出为可分发底包）」在阶段 2 走对象存储上传管道。
> 关联：[remote-vm-control.md](remote-vm-control.md)、[novnc-design.md](novnc-design.md)、
> `room_agent/crates/qe-agent-node/src/vm_control.rs`、`room_node/migrations/0031_machine_command_results.sql`

## 1. 目标（对应“云服务器备份/快照”心智）

教师/管理员在 Web 控制台可以：
1. 选一个**系统底包**（已发布并下发到房间的 VM 磁盘镜像）批量创建虚拟机；
2. 每台新虚拟机使用 **qcow2 差分盘**（backing file 指向底包），只保存增量；
3. 对虚拟机创建**持久化快照**（关机状态下 qcow2 内部快照），可列表/恢复/删除；
4. 查看磁盘链（底包 → 差分盘 → 快照），并把它作为后续“制作镜像”的输入。

## 2. 磁盘模型

```
底包（只读）        <libvirt pool>/<base-volume>.qcow2      ← 由现有镜像管道导入（qe-agent-node Importer）
  └─ 实例差分盘     <data-dir>/vms/<vm>/disk.qcow2          ← qemu-img create -f qcow2 -b <base> -F qcow2
       └─ 快照      qcow2 internal snapshot（快照名）
```

- **底包不修改**：`vm create --base-volume <name>` 只读引用；导入/校验仍走现有镜像管道
  （`image.pull` → `importer/vm.rs` 落 libvirt 卷）。
- **差分盘**：默认 `<data-dir>/vms/<vm>/disk.qcow2`，虚拟大小 `disk_gb`（默认 20G，稀疏文件）。
- **持久化快照**：`qemu-img snapshot -c/-l/-a/-d`，落在差分盘内部，重启/迁移不丢；
  要求虚拟机 **shut off**（运行中拒绝，避免 qcow2 元数据不一致）。
- **优化传输**：同一底包在房间内只分发一次；N 台新机器只产生各自小体积差分盘。
  后续“制作镜像”导出的是**扁平化后的完整 qcow2**（qemu-img convert），可进入现有
  分片+manifest 管道再次分发。

## 3. `vm.command` 协议扩展（阶段 1.5）

| op | args | data |
|----|------|------|
| `create` | `{name, vcpus=2, memory_mib=2048, disk_gb=20, base_volume?, base_path?, gpu?[], start?}` | `{name, disk_path, base, defined:true, started}` |
| `snapshot.create` | `{name, snapshot, description?}` | `{name, snapshot}` |
| `snapshot.list` | `{name}` | `{snapshots:[{tag, created_at, vm_size, date, ...}]}` |
| `snapshot.restore` | `{name, snapshot}` | `{name, snapshot}` |
| `snapshot.delete` | `{name, snapshot}` | `{name, snapshot}` |
| `disk.info` | `{name}` | `{disks:[{path, format, virtual_size, backing_path, backing_format}], chain:[...]}` |
| `export` | `{name, format:"qcow2"}` | `{path, sha256, size_bytes}`（导出到 `<data-dir>/exports/`，完成后可进入镜像上传管道） |

约束：
- `base_volume` 与 `base_path` 二选一；`base_volume` 用 libvirt 存储池卷（默认池
  `LIBVIRT_POOL`，否则 `default`），通过 `virsh vol-path` 解析；
- `name` 只允许 `[A-Za-z0-9._-]{1,64}`（防路径注入）；
- `gpu` 沿用阶段 1 的 PCI 直通格式（`dddd:bb:dd.f`），最多 2 个；
- `snapshot.*` 与 `export` 均要求目标域 shut off（运行中返回明确错误）；
- 所有 op 仍走现有 `machine_command_results` 结果回传与 120s 超时逻辑。

## 4. 控制面 API（阶段 1.5）

1. **单机操作**：沿用 `POST /admin/machines/{serial}/vm/commands`，op 白名单扩展为
   `list|start|shutdown|reboot|destroy|screenshot|create|snapshot.create|snapshot.list|
   snapshot.restore|snapshot.delete|disk.info|export`；`args` 原样校验后透传给 agent。
2. **教师批量下发环境**：新增
   `POST /admin/rooms/{room_id}/vm/environments`（权限 `vm:control`）：
   body `{name_prefix, base_volume, vcpus, memory_mib, disk_gb, auto_start?}`；
   对房间内每个设备槽位（主设备 + `room_devices` 子机）各下发一条 `create` 命令，
   返回 `{"room_id":..., "targets":[{"device_slot":..., "request_id":...}]}`。
   命名：`<name_prefix>-<slot尾号>`（清洗后），冲突时 agent 返回 error，前端提示。
3. **快照持久化**：快照保存在差分盘内，控制面只记录命令结果（不复制数据）；
   阶段 2 可把“从 VM 制作底包”编排为 `export` → presigned PUT → 单分片 manifest → 发布镜像。

## 5. 前端（教师/控制台）

- 机器详情「虚拟机」面板：
  - 新建虚拟机弹窗：名称、vCPU、内存、磁盘、底包（从已发布 `vm_disk` 镜像选择
    volume 名）、是否启动、GPU（可选）；
  - 每台 VM 行内：快照抽屉（创建/列表/恢复/删除）、磁盘链摘要、导出（阶段 2 再接上传）。
- 教师工作台新增「下发环境配置」卡：选房间 + 底包 + 规格 → 批量下发；
  展示每个槽位的 request_id/状态（复用轮询逻辑）。
- 制作镜像：阶段 1.5 先展示“导出磁盘（待上传）”入口，阶段 2 打通上传/发布后变成完整闭环。

## 6. 验收标准（阶段 1.5）

- [ ] `vm.command create` 用 libvirt 卷作 backing file 创建差分盘并 define（可 --start）；
- [ ] 差分盘 `qemu-img info --backing-chain` 能看到底包；
- [ ] 关机状态可创建/列表/恢复/删除 qcow2 内部快照；运行中操作返回明确错误；
- [ ] `disk.info` 返回 backing chain；`export` 产出完整 qcow2 + sha256；
- [ ] 管理端 API 白名单/参数校验单测；前端创建/快照交互 build+vitest 通过；
- [ ] 教师批量下发：一个房间 N 个槽位生成 N 条 pending 命令并可轮询结果；
- [ ] 真机联调：本机 qemu-img + LIBVIRT_URI=qemu:///session 全流程（不碰生产域）。

## 7. 明确不在本阶段

- noVNC 实时隧道（阶段 2，见 novnc-design.md）；
- 底包自动上传到对象存储并发布（阶段 2，需预签名上传+单分片 manifest）；
- 快照跨主机迁移/增量传输（阶段 3，与 P2P 差分分发结合）；
- GPU 真机直通验收（需 IOMMU 环境）。

## 8. 阶段 2 设计：从 VM 制作镜像（导出→上传→发布）

> 本阶段目标是把 `vm.command export` 产出的扁平 qcow2 变成可分发底包。
> 现有 `image_artifacts/image_chunks` 管道按 **chunk** 分发；单文件 qcow2 用
> **单分片 manifest** 即可复用整条分发链路（无需改 agent 侧 pull/import 逻辑）。

### 8.1 数据流

```
教师点「制作镜像」
  └─ qe-api: POST /admin/machines/{serial}/vm/commands {op:"export"} → 等结果 path/sha256/size
       └─ agent: qemu-img convert → <data-dir>/exports/<vm>-<ts>.qcow2
       └─ agent: POST {DEVICE_PKI_MTLS}/machine/images/presign-upload {sha256,size}
            └─ device-pki: 校验 → storage.presign_put(object_key=objects/<sha256>) → 返回 PUT URL
       └─ agent: PUT 文件到 presigned URL（短 TTL）
       └─ agent: POST /machine/images/register {name,version,platform,sha256,size,base_volume?}
            └─ device-pki: HEAD 对象核对 size/sha256 → 构造单分片 manifest →
               INSERT image_artifacts(status=uploaded) + image_chunks(index=0,offset=0)
       └─ qe-api: POST /admin/images/{id}/publish（教师已有 image:manage）
```

### 8.2 新契约（阶段 2）

- `POST /machine/images/presign-upload`（device mTLS）
  body `{sha256, size_bytes}`；sha256 64 位 hex，size ≤ 32GiB；
  object_key = `objects/<sha256>`；返回 `{upload_url, object_key, expires_at}`；
  同一 sha 已存在且 size 一致时可返回 `{already_exists:true}` 让 agent 跳过上传。
- `POST /machine/images/register`（device mTLS）
  body `{name, version, platform, sha256, size_bytes, base_volume?}`；
  校验对象存在、size 一致、magic 为 qcow2（`QFI\xfb`）；构造单分片 manifest：
  `{schema:1, kind:"vm_disk", name, version, platform, format:"qcow2",
    compression:"none", total_size, chunks:[{index:0, object_key:"objects/<sha>",
    sha256, size, offset:0}], runtime:{...}, annotations:{}, created_at, created_by}`；
  `manifest_digest = sha256(manifest_json)`；返回 `{image_id, manifest_digest}`。
- 发布/分配沿用现有 `/admin/images/{id}/publish` 与 assignment 流程，底包随
  `image.pull` 到房间机器后由现有 `VmImporter` 落 libvirt 卷；`vm create --base-volume`
  即可用其做差分盘。**传输优化点**：同一底包在房间内只拉一次，N 台机器仅各自增量盘。

### 8.3 安全与限制

- 上传对象必须由设备 mTLS 身份发起，且 register 的 sha256/size 必须与对象一致；
- 限制并发上传（每设备 1 个）与大小（≤32GiB），防止控制面被打爆；
- 不信任 agent 的 name/version（服务端校验字符集/长度）；
- 阶段 2 不做跨机房底包传输优化（交给 P2P 阶段 3 的多 seeder）。

### 8.4 验收（阶段 2）

- [ ] 从一台关机 VM 导出 qcow2 → presign 上传 → register → publish → 另一台机器
      `image.pull` 后 `vm create --base-volume` 成功；
- [ ] 同一镜像第二次制作命中 `already_exists`，不重复上传；
- [ ] 非 qcow2/size 不符/非法 sha 的 register 被拒绝；
- [ ] 审计记录 制作/上传/发布 三个动作；失败可重试且幂等。

### 8.5 实施定稿（2026-11-07 深夜，阶段 2 落地版本）

- **分片**：agent 把导出 qcow2 按 **32 MiB** 切分（远小于 S3/verify 的 64 MiB get_bytes 上限），
  每片独立 `objects/<sha256>`；manifest 由 device-pki 在 register 时构造并签名，
  因此复用现有 `image.chunks` + `images/{id}/verify` + `publish` 全链路。
- **设备侧端点**（mTLS，agent 调用）：
  - `POST /machine/images/presign-upload` body `{sha256,size_bytes}` →
    `{object_key, upload_url, expires_at, already_exists}`；对象已存在时跳过上传。
  - `POST /machine/images/register` body
    `{name,version,platform,kind:"vm_disk",format:"qcow2",compression:"none",
      total_size, parts:[{sha256,size}], annotations?}` →
    `{image_id, manifest_digest, status:"manifest_ready"}`；
    校验每个 part 对象存在且 HEAD 大小一致、parts 总数 ≤2048、单 part ≤64 MiB、
    sum(part.size)==total_size；name/version/platform 复用 domain 校验；
    (name,version,platform) 重复 → 409。
- **agent 命令**：`vm.command` 新增 op `image.make`
  args `{vm_name, name, version, platform?, base_volume?, description?}`；
  要求 VM shut off；流程 = qemu-img 导出 → 32MiB 分片 → 逐片 presign/PUT（已存在跳过）
  → register（device-pki 签名 manifest）→ 返回 `{image_id, manifest_digest, total_size, chunks}`；
  超时 600s；失败清理本地导出文件。
- **控制台**：「制作镜像」按钮在 VM 行内（关机才可用）→ 弹窗填镜像名/版本 →
  下发 image.make → 成功后自动调用 `POST /admin/images/{id}/verify` 与
  `POST /admin/images/{id}/publish`，展示最终状态与镜像列表入口。
- **验收（本轮）**：本机 session VM 导出 → 上传 rustfs → register → verify → publish →
  `/images` 可见且可分配；失败用例：qcow2 非镜像/magic 错误在 verify 阶段被拒。
