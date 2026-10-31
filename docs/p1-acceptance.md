# P1 真机联调 / 验收清单

> 目标：在真实进程、真实设备证书、真实 RustFS、真实容器/虚拟机运行时下跑通
> `镜像上传 → 验证 → 分配房间 → device-pki 下发 → agent 拉取校验导入 → 状态回写`。
> 单元/集成测试已覆盖的部分见 §1，本清单只列需要真机/真实服务验证的项。

## 1. 已自动化验证（CI/本地即可）

| 项 | 位置 |
| --- | --- |
| S3 SigV4 预签名（AWS 官方向量） | `cargo test -p qe-storage` |
| 上传/注册/manifest/verify/publish/分配（fake S3） | `crates/api/tests/storage.rs` |
| device-pki 预签名授权、transfer 状态回写（真实 PG） | `crates/device-pki/tests/{resource_presign,transfer_flow}.rs` |
| agent manifest 验签、拉取校验、缓存、job 编排（mock 网络/导入器） | `cargo test -p qe-node-agent` |
| 三个导入器命令构造（fake 可执行脚本） | `cargo test -p qe-node-agent importer` |

## 2. 待真机验证

- [ ] RustFS 真实读写 + 预签名 PUT/GET（已在本机验证一次，见 README）
- [ ] device-pki 真实 mTLS（证书签发 → 设备确认激活 → `/resources/presign` 授权通过）
- [ ] agent 通过 device-pki 收到 `image.pull` 并完成拉取
- [ ] Docker/OCI 导入（`docker load` / `ctr images import`）
- [ ] Incus 导入 或 plain LXC rootfs 解包
- [ ] libvirt VM 卷创建 + 上传
- [ ] `transfers` 状态 `pending → running → completed`，`image_assignments.rollout_state=deployed`

## 3. 环境准备

### 3.1 服务端

1. RustFS（compose profile）：`docker compose --profile storage up -d rustfs`，创建 bucket `qestudio`。
2. qe-api：`.env` 配好 `S3_*` + `STORAGE_SIGNING_KEY_BASE64`，`cargo run -p qe-api`，确认 `/readyz` 200。
3. qe-device-pki：生成/准备 CA 与服务端证书（`cargo run -p qe-device-pki --example gen_test_ca` 或现场 CA），
   **export 与 qe-api 相同的 `S3_*` 和同一个 `STORAGE_SIGNING_KEY_BASE64`**，启动 enroll/mTLS 两个 listener。

### 3.2 节点

- 一台节点（或本机模拟）具备：device-pki 客户端证书、`docker` 或 `ctr`、按需 `incus`、`virsh`/libvirt pool、`tar`。
- 若与 device-pki 同机，注意 `127.0.0.1` 可达；跨机需 `DEVICE_PKI_MTLS_URL=https://<host>:4441` 与证书 CN/slot 匹配。

## 4. 联调步骤

### Step 1：设备注册与房间绑定

```bash
# 管理端发一次性注册 token（ADMIN_TOKEN）
curl -s -X POST https://<enroll>:4440/admin/tokens \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'content-type: application/json' \
  -d '{"kind":"auto","slot":"room-101","ttl_secs":900}'
# 设备端注册（qe-device-pki-client CLI；按仓库 README/契约使用 enroll 链接）
# 注册成功后确认：GET /admin/enrollments?status=active 看到 room-101
# 房间绑定槽位：
curl -s -X PUT http://<api>/api/v1/admin/rooms/<room_id>/device \
  -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"device_slot":"room-101"}'
```

期望：房间 `device` 显示 serial/active/online，`pending_token=false`。

### Step 2：镜像产物上传、验证、发布、分配

```bash
# 以已有镜像为例（真机任选一个可 docker load 的 tar）
docker save redis:7-alpine -o /tmp/image.tar
# 按 README「资源与镜像管理（P0）」：
#   presign 分片 → curl PUT → 注册对象 → 提交 manifest → verify → publish → 分配房间
```

期望：`GET /api/v1/admin/images/{id}` 为 `published`；分配后 `device_downlink` 出现 `kind=image.pull`。

### Step 3：agent 启动与导入

```bash
DEVICE_PKI_MTLS_URL=https://<mtls>:4441 DEVICE_SLOT=room-101 \
DEVICE_DATA_DIR=/var/lib/qe-agent CACHE_DIR=/var/lib/qe-agent/cache \
DOCKER_BIN=docker RUST_LOG=info,qe_node_agent=debug \
cargo run -p qe-node-agent
```

期望日志：`manifest verified` → `fetched chunks` → `import ok` → 上报完成；
节点上 `docker images` 出现对应镜像。

### Step 4：状态与证据

```bash
# API 查询
curl -s "http://<api>/api/v1/admin/transfers?room_id=<room_id>" -H "Authorization: Bearer $TOKEN"
# DB 抽查
psql "$DATABASE_URL" -c "SELECT id,status,progress,error FROM transfers ORDER BY created_at DESC LIMIT 1;"
psql "$DATABASE_URL" -c "SELECT image_id,room_id,rollout_state FROM image_assignments;"
```

通过标准：

| 检查 | 期望 |
| --- | --- |
| device-pki `/resources/presign` | 200，返回 manifest/chunk URL + public key |
| agent | 校验通过、导入成功、无 panic |
| `transfers.status` | `completed`，`progress=100`，`error IS NULL` |
| `image_assignments.rollout_state` | `deployed` |
| 节点运行时 | `docker images` / `incus image list` / `virsh vol-list <pool>` 存在对应产物 |

### Step 5：失败路径

- 把房间设备解绑/设备离线 → 分配后 agent 收不到消息，`transfers` 保持 `pending`。
- 篡改一个分片（直接改 S3 对象）→ verify 或 agent 校验失败，`transfer.completed{ok:false}`，
  `rollout_state=failed`，节点保留上一版本。
- 配置错误签名密钥 → manifest 验签失败（agent 拒绝，服务端 verify 也会失败）。

## 5. 常见问题

| 现象 | 排查 |
| --- | --- |
| `/resources/presign` 403 | 设备 slot 未绑定房间 / 镜像未分配到该房间 |
| 503 storage_unavailable | device-pki 进程缺少 `S3_ENDPOINT`/`S3_BUCKET`（它不读 `.env`） |
| manifest 验签失败 | qe-api 与 device-pki 的 `STORAGE_SIGNING_KEY_BASE64` 不一致或重启换随机 key |
| agent 连不上 | `DEVICE_PKI_MTLS_URL`、证书 CN 与 `DEVICE_SLOT`、CA 信任、4441 端口 |
| docker load 失败 | `DOCKER_BIN` 权限/socket；镜像 tar 是否完整（sha256/size 校验先过） |
| Incus 未安装 | `INCUS_BIN` 置空触发 plain LXC 解包（需 root 写 `LXC_ROOTFS_DIR`） |
| virsh 失败 | `LIBVIRT_POOL` 是否存在、libvirt 权限、磁盘空间 |
