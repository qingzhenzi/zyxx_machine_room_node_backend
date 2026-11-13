# 启动配置与自动环境拉起（startup provisioning）契约 v1

> 状态：**设计定稿，服务端 + agent 并行实现中**（2026-11-13）。
>
> 背景：现在「设备启动虚拟机」需要人工串起三步：管理端分配镜像 → `image.pull` 下行
> → 再人工下发 `vm.command create`。远程机（如 `ssh -p 2222 root@192.168.8.141`）
> 部署 agent 后没有任何"连上服务端就自动准备环境"的能力。本契约补上这条链：
>
> **agent 连上服务端 → 拉启动配置 → 缺镜像自动拉 → 缺 VM 自动建 → 按 auto_start 启动。**
>
> 该契约是 `room_node`（服务端）与 `room_agent`（agent）两个仓库的共同依据。
> 两端各自有 JSON 类型定义，字段名/语义以本文为准，改动必须同步两端与本文。

## 1. 角色与数据流

```text
管理端 / 教师端
  PUT /api/v1/admin/rooms/{room_id}/startup   （保存启动配置，revision+1）
        └─ enqueue downlink  kind=startup.update  {"revision":N}
              └─ device-pki GET /messages/downlink ──► qe-agent
                                                          │
qe-agent 启动 / 收到 startup.update / 周期 reconcile        │
  GET {DEVICE_PKI_MTLS_URL}/agent/startup   （设备证书 mTLS）◄┘
        │  {revision, enabled, auto_pull, auto_start_vm, images[], vms[]}
        ▼
  ┌─ images[]：本机没有该镜像 → 复用 image.pull 链路（P2P/HTTP → 校验 → import）
  ├─ vms[]：域不存在 → vm_control create（差分盘 + define）
  │         域存在但关机且 auto_start → vm_control start
  └─ 写 <data_dir>/startup-state.json（revision / 已导入 digest / VM 动作）
```

安全边界：

* agent 只做「确保存在 + 启动」，**绝不删除、关机、重启**启动配置之外的 VM；
* 服务端只返回 `published` 状态且 manifest 完整的镜像；
* 自动创建 VM 使用只读底包 + qcow2 差分盘，底包不复制、不修改。

## 2. 服务端存储

迁移 `room_node/migrations/0035_room_startup_profiles.sql`：

```sql
CREATE TABLE IF NOT EXISTS room_startup_profiles (
    room_id UUID PRIMARY KEY REFERENCES rooms(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    revision BIGINT NOT NULL DEFAULT 1,
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

`config` 形状（管理端写入，服务端校验后落库）：

```json
{
  "auto_pull": true,
  "auto_start_vm": true,
  "images": [{ "image_id": "204cb75d-ac1b-4492-b8b3-808d2270070b" }],
  "vms": [
    {
      "name": "lab-104",
      "vcpus": 2,
      "memory_mib": 2048,
      "disk_gb": 20,
      "base_volume": "real-debian-13-0.1.1",
      "base_path": null,
      "gpu": [],
      "auto_start": true
    }
  ]
}
```

校验规则（API 返回 400，错误信息指出字段）：

| 字段 | 规则 |
|---|---|
| `images` | 数组 0..=8；`image_id` 为 UUID；镜像必须存在、`status=published`、`manifest_digest` 非空 |
| `images[].image_id` | 去重后写入 |
| `vms` | 数组 0..=32 |
| `vms[].name` | `^[A-Za-z0-9._-]{1,64}$`，去重 |
| `vms[].vcpus` | 1..=64，缺省 2 |
| `vms[].memory_mib` | 256..=262144，缺省 2048 |
| `vms[].disk_gb` | 1..=2048，缺省 20 |
| `vms[].base_volume` | 可选，`^[A-Za-z0-9._-]{1,128}$`；与 `base_path` 互斥 |
| `vms[].base_path` | 可选，必须是绝对路径；与 `base_volume` 互斥 |
| `vms[].gpu` | ≤2 个，格式 `^[0-9a-fA-F]{4}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-7]$` |
| `vms[].auto_start` | bool，缺省 true |
| `auto_pull` / `auto_start_vm` | bool，缺省 true |

`base_volume` 缺省语义：若 `vms[].base_volume` 与 `base_path` 都为空，且 `images` 中恰好
有一个 `kind=vm_disk` 的镜像，服务端不填；**agent 侧**按下式推导
`<image.name>-<image.version>`（与 `importer/vm.rs` 的卷名规则一致）。多个 vm_disk
镜像时必须显式给 `base_volume`/`base_path`，否则 agent 跳过该 VM 并告警。

## 3. 管理端 API（qe-api）

### 3.1 GET `/api/v1/admin/rooms/{room_id}/startup`（权限 `vm:control`）

```json
{
  "room_id": "8f846cb5-7d5e-4181-8c9c-631b7a112397",
  "enabled": true,
  "revision": 3,
  "config": { "...同 §2 config..." },
  "updated_at": "2026-11-13T05:00:00+00:00",
  "images": [
    {
      "image_id": "204cb75d-...",
      "name": "real-debian-13",
      "version": "0.1.1",
      "kind": "vm_disk",
      "status": "published",
      "manifest_digest": "sha256:...",
      "total_size": 2148073472,
      "transfer_id": "a105a24f-..."
    }
  ],
  "targets": [{ "device_slot": "dev-b104", "serial": "0638..." }]
}
```

无配置行时返回 `enabled=false, revision=0, config={auto_pull:true,auto_start_vm:true,images:[],vms:[]}`（200）。

### 3.2 PUT `/api/v1/admin/rooms/{room_id}/startup`（权限 `vm:control`）

请求体：

```json
{ "enabled": true, "config": { "...§2 config..." } }
```

行为：

1. 校验房间存在（404）、config 合法（400）、镜像存在且已发布（400）；
2. upsert：首次 `revision=1`，之后 `revision+1`，写 `updated_by`；
3. 保证每个 `images[].image_id` 在该房间有 `image_assignments` 行（幂等 upsert；
   `authorize_image` 要求镜像已分配到房间，否则 agent 预签名 403）与至少一条 `transfers`
   记录（`direction='download'`，`status='pending'`，已有则复用最新一条），供 agent
   上报 `transfer.progress/completed`；
4. 向房间主槽位（`rooms.device_slot`）+ `room_devices` 各槽位入队
   `device_downlink(kind='startup.update', payload='{"revision":N}')`，并发布长连接唤醒
   （失败只 warn）；
5. 返回与 GET 相同结构。

## 4. 设备 mTLS 接口（device-pki）

### 4.1 GET `/agent/startup`（设备证书 mTLS，无请求参数）

身份：设备证书 serial → `device_enrollment(issued|active)` → `device_slot` →
`rooms.device_slot` 或 `room_devices` → room。无存活注册 → 401。

响应（无配置 / `enabled=false` 时也返回 200，`revision=0`）：

```json
{
  "revision": 3,
  "enabled": true,
  "auto_pull": true,
  "auto_start_vm": true,
  "images": [
    {
      "image_id": "204cb75d-ac1b-4492-b8b3-808d2270070b",
      "transfer_id": "a105a24f-cc3e-48dc-ad37-2c3368a36829",
      "name": "real-debian-13",
      "version": "0.1.1",
      "kind": "vm_disk",
      "format": "qcow2",
      "platform": "debian/amd64",
      "manifest_digest": "sha256:...",
      "total_size": 2148073472,
      "chunk_count": 65,
      "presign_endpoint": "/resources/presign"
    }
  ],
  "vms": [ "…同 §2 vms，base_volume 可为 null…" ]
}
```

* `images[]` 只包含已发布且 manifest 完整的镜像；`transfer_id` 取该房间该镜像最新
  transfer，没有则 `null`（agent 用本地随机 UUID 上报，服务端查不到即忽略）；
* 字段名与 `image.pull` payload 保持一致（`name/version/kind/format/platform/
  manifest_digest/total_size/chunk_count/presign_endpoint`），agent 可直接转成
  `image.pull` payload 复用现有 `ImagePullJob`。

### 4.2 下行 `startup.update`（device_downlink kind）

payload：`{"revision":N}`。agent 收到后立即触发一次 reconcile（不做 ack 语义特殊处理；
reconcile 本身幂等，重复/漏收都由周期 reconcile 兜底）。

## 5. agent 行为（room_agent）

### 5.1 开关与节奏（env）

| 环境变量 | 默认 | 说明 |
|---|---|---|
| `QE_STARTUP_ENABLED` | `1` | `0/false/no/off` 关闭启动配置拉取（仅保留下行/人工命令） |
| `QE_STARTUP_RECONCILE_SECS` | `60` | 周期 reconcile 间隔，0..=3600；`0` 表示只在启动/收到 `startup.update` 时执行 |
| `QE_STARTUP_HTTP_TIMEOUT_SECS` | `10` | GET `/agent/startup` 超时 |

### 5.2 时机

1. `qe-agent serve --role node|both` 启动、mTLS confirm 与远端策略拉取之后，**先做一次
   reconcile**（失败只 warn，不阻塞下行轮询）；
2. 每 `QE_STARTUP_RECONCILE_SECS` 周期执行一次；
3. 收到下行 `startup.update` 后立即执行一次（与周期任务串行，不并发 import）。

### 5.3 reconcile 语义（幂等）

1. `GET {DEVICE_PKI_MTLS_URL}/agent/startup`；`enabled=false` 或 `revision=0` → 记日志退出；
2. 镜像（`auto_pull=true` 时）：
   * 先判本机是否已有该镜像：
     - `vm_disk`：`virsh vol-list <LIBVIRT_POOL>` 存在 `<name>-<version>`；
     - `docker`/`oci`：`docker image inspect <ref>` 成功（ref 取 manifest runtime ref，
       回退 `<name>:<version>`）；
     - `lxc_rootfs`：本机 ledger 有该 digest，或 `incus image list --format csv` 含 `name`；
     - 其它/探测失败：以 ledger 为准；无 ledger 视为缺失；
   * 已有 → 跳过（debug 日志）；
   * 缺失 → 用 §4.1 的字段构造 `image.pull` payload，复用 `ImagePullJob`
     （P2P 优先、HTTP 回退、逐片校验、import 全部不变）；失败 → warn 并在下次周期重试；
   * 与下行 `image.pull` 共用同一把 import 锁，串行导入；
3. VM（`auto_start_vm` 与 `vms[].auto_start` 都需为 true 才自动启动）：
   * 先 `vm_control list` 查域状态；
   * 不存在 → `create`（args：name/vcpus/memory_mib/disk_gb/base_volume|base_path/gpu，
     `start = auto_start`）；底包卷缺失（镜像尚未拉完/拉取失败）→ warn 后跳过，下轮重试；
   * 存在且 `running` → 不动；
   * 存在且 `shut off` 且允许自动启动 → `start`；
   * 其它状态（paused/crashed）只告警，不擅自操作；
4. 每轮结束写 `<data_dir>/startup-state.json`：

```json
{
  "applied_revision": 3,
  "updated_at": "2026-11-13T05:01:02+00:00",
  "images": { "sha256:...": "libvirt:default/real-debian-13-0.1.1" },
  "vms": { "lab-104": "started" }
}
```

5. 日志：`startup 拉取成功 revision=…`、`镜像已存在，跳过 name=…`、
   `开始拉取镜像 name=…`、`自动创建并启动 VM name=…`、`VM 已在运行，跳过 name=…` 等，
   统一带 `startup` 关键字便于 `journalctl -u qe-agent | grep startup`。

### 5.4 命令行辅助

`qe-agent cli startup`（只读，便于远程排障）：

```bash
qe-agent cli startup show   # GET /agent/startup 原文 + 本机 startup-state.json
qe-agent cli startup apply  # 立即执行一次 reconcile（等价 startup.update）
```

## 6. 远程部署（SSH）

配套脚本 `room_agent/scripts/deploy-remote-agent.sh`（见 `docs/agent-deploy.md` 远程机章节）：

```bash
sudo bash scripts/deploy-remote-agent.sh \
  --host 192.168.8.141 --port 2222 --user root \
  --server https://192.168.8.231:4441 \
  --slot dev-b104 \
  --enroll-link "https://192.168.8.231:4440/enroll?token=…&slot=dev-b104" \
  --password '***'          # 或用 SSH 密钥 / SSHPASS 环境变量
```

脚本负责：传二进制 + systemd 单元 → 合并写 `/etc/qe-agent.env`（**强制
`DEVICE_PKI_MTLS_URL` 指向控制节点 LAN 地址，不再用 localhost**）→ 可选 enroll →
`systemctl enable --now qe-agent` → 通过本机 IPC `/v1/status` 打印自检结果。

## 7. 验收标准

1. `cargo test -p qe-device-pki -j4` / `cargo test -p qe-agent-node -j4` / 相关 api、db 测试通过；
2. 远程机（192.168.8.141）`qe-agent cli status` 显示 `mtls_endpoint=https://<控制节点LAN>:4441`、enrolled；
3. 管理端 PUT 启动配置（镜像 + `lab-104` VM，auto_start）后，仅重启 agent（不手工下发
   `image.pull` / `vm.command`），远程机在 60s 内出现 `lab-104` 且 state=running，
   `startup-state.json` 记录 revision 与 VM 动作；
4. 重复重启 agent / 重复 reconcile 不重复建盘、不重复拉镜像、不重启运行中的 VM；
5. 删除 `lab-104` 后再重启 agent，能被重新创建并启动；
6. 远程部署脚本可在 192.168.8.141:2222 上一次跑通（二进制 + env + enroll + 启动 + 自检）。
