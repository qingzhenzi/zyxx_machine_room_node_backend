# 资源与镜像管理（Storage / Image）设计草案 v0.1

> 状态：**P0（后端）与 P1（device-pki 扩展 + 房间节点 agent）已实现**；真实 RustFS/mTLS/Incus/libvirt 端到端待环境联调。目标是为房间节点提供「资源分发 / 收集」与「镜像（Docker/OCI、LXC rootfs、VM 磁盘）管理」的独立模块。
> 数据面用 S3（RustFS / MinIO / Ceph 等 S3 兼容实现），控制面放在 `qe-api`；**P0 不实现 Registry 协议**。

## 1. 目标与非目标

### 目标

- **资源分发**：按区域 / 房间 / 课表事件，把文件、镜像、系统底包、分片磁盘下发到房间节点。
- **资源收集**：房间节点把产物（日志、作业、录像、快照等）上传回 S3，后端登记与校验。
- **镜像管理**：Docker/OCI 镜像、LXC rootfs、VM 磁盘（qcow2/raw）的统一元数据、清单（manifest）、校验、版本/通道、分配与回滚。
- **完整性**：每个产物都有 manifest + 分片 sha256 + 签名 + 验证报告；节点侧拉取后再次校验（fail-closed）。
- **可替换存储**：只依赖 S3 API，开发用 MinIO，目标候选 RustFS，生产可换 Ceph/云 OSS。

### 非目标（P0 明确不做）

- 不实现完整 OCI Registry（push/pull/GC/鉴权）；需要标准 `docker pull` 时的方案见 §9。
- 不做实时/懒加载（stargz/nydus/SOCI）、不做 VM 远程流式启动。
- 不做容器编排/服务健康管理（那是房间节点 agent/编排层的职责）。
- 不做多租户计费。

## 2. 运行时现状与适配

房间节点主要运行 **Docker 容器、LXC 容器、普通 KVM/虚拟机**。三者消费镜像的方式不同，但可以用同一套「manifest + 分片 + 校验 + 导入适配器」统一：

| 类型 | 产物格式 | 节点侧导入方式 | 备注 |
| --- | --- | --- | --- |
| Docker / OCI | `docker-archive tar` 或 `oci-layout tar`（可 zstd） | `docker load -i` / `ctr images import` / `skopeo copy` | 不做 Registry；全量下载后导入 |
| LXC | rootfs `tar`（可 zstd）；或 LXD/Incus 镜像 tar | plain LXC：解包到 `/var/lib/lxc/<name>/rootfs`；LXD/Incus：`lxc image import` | LXC 本身没有 registry 概念 |
| VM（KVM/libvirt） | `qcow2` / `raw` 磁盘（按 chunk 切分） | 分片校验后组装为 qcow2/raw，放入 libvirt storage pool（`virsh vol-upload` 或直接落盘） | 系统底包与实例分离；实例由 libvirt 管理 |

统一抽象：

```text
manifest (清单) + content-addressed chunks + verification report
        │
        ├── kind=docker       → node agent: docker load
        ├── kind=oci          → node agent: ctr images import
        ├── kind=lxc-rootfs   → node agent: 解包 / lxc image import
        └── kind=vm-disk      → node agent: 组装 qcow2/raw → libvirt
```

**分片复用**：同一 base 镜像的多个版本共享 chunk（内容寻址），节点只补缺失 chunk，实现增量更新，无需 Registry。

## 3. S3 存储布局

```text
<bucket>/
  objects/<sha256>                      # 单个 blob / chunk（内容寻址）
  manifests/<manifest_sha256>.json      # 清单
  signatures/<manifest_sha256>.sig      # 清单签名（Ed25519）
  collections/<area_id>/<room_id>/...   # 收集上来的产物（按来源分区）
  tmp/<upload_id>/...                   # 分片上传临时区（生命周期规则清理）
```

- 所有对象以 sha256 内容寻址，重复内容天然去重；上传/下载用短 TTL 预签名 URL。
- 分片大小默认 16 MiB（可配，`IMAGE_CHUNK_SIZE_BYTES`），压缩默认 zstd。
- 支持 Range 下载与断点续传；节点侧缓存已校验 chunk，LRU 淘汰。

## 4. 清单（manifest）格式

```json
{
  "schema": 1,
  "kind": "docker",                  // docker | oci | lxc-rootfs | vm-disk
  "name": "web-console",
  "version": "2026.10.31-1",
  "platform": "linux/amd64",
  "format": "docker-archive",       // docker-archive | oci-layout | tar | qcow2 | raw
  "compression": "zstd",
  "total_size": 734003200,
  "chunks": [
    { "index": 0, "digest": "sha256:ab12…", "size": 16777216, "offset": 0 }
  ],
  "runtime": {
    "docker": { "image_ref": "web-console:2026.10.31-1" },
    "lxc": { "distro": "ubuntu", "release": "24.04" },
    "vm": { "os": "windows-11", "disk_size": 34359738368, "format": "qcow2" }
  },
  "annotations": {},
  "signature": "ed25519:…",           // 对去掉本字段后的 JSON 规范化签名
  "created_at": "2026-10-31T05:00:00Z",
  "created_by": "user-id"
}
```

- `version` 只是展示/排序用；**权威标识是 `manifest_sha256`**（可作 image digest）。
- 发布时对 manifest 签名；节点先验签 manifest，再逐 chunk 校验 sha256。
- 容器镜像若从构建系统产出，直接按 `docker save` / `skopeo copy docker-archive:` 后切分；保留原镜像 ID 到 annotations。

## 5. 数据模型（PostgreSQL）

```text
storage_objects(id, bucket, object_key, sha256, size, content_type, created_at)
image_artifacts(id, name, version, kind, format, compression, platform,
                manifest_digest, total_size, status, created_by, created_at, updated_at)
image_chunks(manifest_digest, idx, object_key, sha256, size, offset)
image_verifications(id, artifact_id, status, report JSONB, verified_by, verified_at)
image_channels(id, artifact_id, channel, is_current)            -- dev/test/prod
image_assignments(id, image_id, area_id, room_id, channel,
                  rollout_state, assigned_by, assigned_at, updated_at)
resource_bindings(id, resource_type, resource_id, area_id, room_id,
                  schedule_id, event_rule_id, created_at)
transfers(id, direction, node_slot, room_id, kind, object_key, manifest_digest,
          status, progress, attempts, error, expires_at, created_at, updated_at)
collections(id, area_id, room_id, node_slot, kind, object_key, sha256, size,
            source_event_id, uploaded_by, status, created_at)
```

状态机：

- artifact：`uploaded → manifest_ready → verifying → verified → published → deprecated/revoked`
- transfer：`pending → running → completed / failed / expired`
- collection：`pending → uploaded → verified / rejected`

## 6. 验证流水线（清单验证）

发布前必须 `verified`；验证报告落 `image_verifications.report`（JSON），可供审计。

| 检查 | 适用 | 说明 |
| --- | --- | --- |
| 分片完整性 | 全部 | 重算每个 chunk sha256/size，与 manifest 一致 |
| 总大小一致 | 全部 | sum(chunk.size) == total_size |
| 格式 magic | 全部 | docker-archive/oci-layout/tar/qcow2/raw 的魔数/目录结构 |
| 压缩可解 | 全部 | zstd/gzip 解压并校验尾部 |
| 清单签名 | 全部 | Ed25519 验签（`signatures/<digest>.sig`） |
| 运行时冒烟（可选） | docker/oci | 隔离环境 `docker load` 或 `ctr images import` + `--help` 不执行 |
| LXC rootfs 检查 | lxc-rootfs | tar 结构含 `/etc`、`/bin/sh` 等基础路径 |
| 磁盘检查 | vm-disk | `qemu-img info` / `qemu-img check`（qcow2），可选快照启动冒烟 |
| 安全扫描（可选） | docker/oci | Trivy / Grype 结果入 report，不阻塞 P0 |

**节点侧校验**：拉 manifest → 验签 → 逐 chunk 校验 → 组装后重算整文件 sha256；任何一步失败即拒绝并使用上一个已验证版本（fail-closed）。

## 7. 分发与收集流程

### 分发（控制面 → 房间节点）

1. 管理员/课表规则把 artifact 分配到 area/room（`image_assignments` 或 `resource_bindings`）。
2. 触发条件：手动、课表事件（上课前 N 分钟 / 房间占用）、批量发布。
3. 后端生成 `transfers`，通过 device-pki 下行发信令（≤64KB）：
   ```json
   { "type": "resource.available", "manifest_digest": "sha256:…",
     "kind": "docker", "manifest_url": "<presigned>", "expires_at": "…",
     "transfer_id": "…" }
   ```
4. 节点 agent 拉 manifest → 验签 → 按 chunk 拉取（预签名 URL / Range）→ 组装/导入 → 上报
   `transfer.completed`（device-pki 上行或 HTTP 回调）。大文件**绝不走下行消息通道**。
5. 失败重试：分片级断点续传；manifest/签名失败不重试，直接标记并告警。

### 收集（房间节点 → S3 → 后端）

1. 后端创建 `collections` 任务并下发 `upload.requested`（含预签名 PUT / 分片规则）。
2. 节点分片上传 → 完成通知 → 后端校验（sha256/size）→ `verified`。
3. 可选 S3 Event Notification 作为完成兜底；审计与保留期/配额在 P2 加。

## 8. API（`qe-api` 新增，全部 `/api/v1/admin`）

| 方法 | 路径 | 权限 | 说明 |
| --- | --- | --- | --- |
| POST | `/storage/uploads` | `storage:manage` | 初始化上传（返回预签名分片 PUT 列表/upload_id） |
| POST | `/storage/uploads/{id}/complete` | `storage:manage` | 完成上传，生成 `storage_objects` |
| POST | `/images` | `image:manage` | 创建 artifact（name/version/kind/format/platform） |
| POST | `/images/{id}/manifest` | `image:manage` | 提交 manifest + chunk 列表，进入验证 |
| POST | `/images/{id}/verify` | `image:manage` | 触发验证（可异步），返回报告 |
| POST | `/images/{id}/publish` | `image:manage` | 仅 verified 可发布；写通道 |
| POST | `/images/{id}/deprecate` | `image:manage` | 废弃/吊销 |
| GET | `/images?kind=&name=&status=&page=` | `image:read` | 镜像分页 |
| GET | `/images/{id}` | `image:read` | 详情 + chunks + 验证报告 |
| GET | `/images/{id}/download-manifest` | `image:read` | 预签名 manifest 下载地址 |
| PUT | `/rooms/{id}/images/{image_id}` | `image:manage` | 房间分配/切换版本 |
| DELETE | `/rooms/{id}/images/{image_id}` | `image:manage` | 取消分配 |
| GET | `/transfers?room_id=&status=&page=` | `storage:read` | 分发/收集任务 |
| POST | `/transfers/{id}/retry` | `storage:manage` | 重试 |
| GET | `/collections?room_id=&status=&page=` | `storage:read` | 收集列表 |
| POST | `/resources` | `storage:manage` | 注册普通资源并绑定房间/课表规则 |

- 权限：新增 `storage:read` / `storage:manage`、`image:read` / `image:manage`；admin 自动补齐，viewer/editor 只读。
- 审计动作：`storage.upload.*`、`image.create/verify/publish/deprecate`、`image.assign/unassign`、`transfer.retry`、`collection.verify`。

## 9. 需要标准 `docker pull` / k8s 时

- P0 不做 Registry。若后续房间节点需要 `docker pull` / `podman pull` / k3s/kubelet 拉取：
  - **优先**：部署 Zot（单二进制，可 S3 后端）作为只读/被推送的 registry；控制面用 `skopeo`/`oras` 把已验证 artifact 推入 Zot。digest 不变，迁移成本低。
  - 或实现**只读 Registry API 子集**（`GET /v2/`、manifests、blobs、HEAD/Range/`Docker-Content-Digest`，可选 tags），后端直接代理 S3；不做 push/GC/UI。
- 两条路都在 P3 评估，不影响 P0/P1。

## 10. 配置（新增）

| 环境变量 | 默认 | 说明 |
| --- | --- | --- |
| `S3_ENDPOINT` | 无（必填） | S3 兼容端点（MinIO/RustFS/Ceph） |
| `S3_REGION` | `us-east-1` | 区域 |
| `S3_BUCKET` | 无（必填） | 桶名 |
| `S3_ACCESS_KEY` / `S3_SECRET_KEY` | 无 | 凭证（生产建议走 Secret） |
| `S3_PATH_STYLE` | `true` | MinIO/RustFS 通常需要 path-style |
| `STORAGE_PRESIGN_TTL_SECONDS` | `900` | 预签名 TTL，60..=86400 |
| `IMAGE_CHUNK_SIZE_BYTES` | `16777216` | 分片大小，1MiB..=256MiB |

## 11. 分阶段计划

- **P0（本仓库）**：`qe-storage` 适配 crate + 迁移 + 镜像/资源/上传/验证/预签名 API + 单元/集成测试；开发用 MinIO（S3 兼容）。
- **P1（节点侧）**：房间节点 agent（建议独立 Rust 二进制）：拉 manifest/chunks、校验、按 kind 导入 Docker/LXC/VM、上报；与课表事件联动（上课前分发）。
- **P2**：base 复用/增量、收集上传、S3 Event 兜底、配额/保留/GC、监控与告警。
- **P3（可选）**：Zot facade 或只读 Registry API；k8s/k3s 支持；Trivy 扫描。

## 12. 待确认

1. S3 实现：开发用 MinIO 可以吗？目标是否先用 RustFS 做 PoC？
2. LXC 是 **plain LXC** 还是 **LXD/Incus**？（决定 rootfs 导入方式：解包 vs `lxc image import`）
3. VM 是否由 **libvirt/KVM** 管理？（决定磁盘落到 storage pool 的方式）
4. 房间节点 agent 形态：独立 Rust 二进制 / 扩展 `device-pki-client` / 外部已有 agent？（决定 P1 的仓库与接口）
5. 典型镜像大小、房间数量、更新频率（决定分片大小、缓存与是否需要增量）。

## 13. P0 验收标准

- `make check` 通过；新增迁移 `0011_storage.sql` 在 `qestudio_test` 应用成功。
- MinIO 下完成：创建 artifact → 分片上传 → 生成 manifest → 验证（含签名/sha256/格式）→ 发布 → 预签名下载 → 校验一致。
- 集成测试覆盖：上传/验证/发布状态机、非法 sha256 拒绝、权限 403、审计记录、房间分配与 transfer 创建、收集登记。
- README 增加「资源与镜像管理」章节；OpenAPI 增加 `Storage`/`Images` tag。

---

## 14. P0 实现契约（v1，开工版）

> 范围：仅本仓库后端。S3 目标实现 **RustFS**（S3 兼容；本地测试受网络限制时用进程内 fake S3，部署配置指向 RustFS）。
> 节点 agent（独立 crate）在 P1；LXC 推荐 Incus，rootfs tar 格式对 plain LXC 也兼容。
> 规则：具体时间点为 `TIMESTAMPTZ`（UTC）；所有 S3 对象内容寻址；manifest 存 `manifests/{sha256}.json`，签名存 `signatures/{sha256}.sig`（Ed25519 对 manifest 原始字节签名）。
> **P0 只接受 `compression = none`**（zstd/gzip 字段保留，非 none 时 manifest 校验返回 400；P1 再实现）。

### 14.1 新增 crate `crates/storage`（包名 `qe-storage`）

加入 workspace members。依赖：`anyhow`、`chrono`、`ed25519-dalek = "2"`、`hmac = "0.12"`、`sha2 = "0.10"`、`hex`、`percent-encoding`、`reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }`、`serde`、`serde_json`、`thiserror`、`uuid`。

```rust
#[derive(Debug, Clone)]
pub struct StorageOptions {
    pub endpoint: String,      // 例如 http://127.0.0.1:9000
    pub region: String,        // 默认 us-east-1
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub path_style: bool,      // RustFS/MinIO 用 true
    pub presign_ttl_secs: u64, // 60..=86400
    pub signing_key_base64: Option<String>, // 32 字节 Ed25519 seed；None = 启动随机生成（仅开发）
}

#[derive(Debug, Clone)]
pub struct PresignedUrl {
    pub url: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

pub struct StorageService { /* S3 客户端 + Ed25519 签名器 */ }

impl StorageService {
    pub fn new(options: StorageOptions) -> anyhow::Result<Self>;
    pub fn bucket(&self) -> &str;
    pub fn public_key_base64(&self) -> String;
    pub fn presign_get(&self, object_key: &str) -> anyhow::Result<PresignedUrl>;
    pub fn presign_put(&self, object_key: &str, content_type: Option<&str>) -> anyhow::Result<PresignedUrl>;
    /// 服务端读取：内部用预签名 GET + reqwest，返回完整字节（P0 对象都很小）。
    pub async fn get_bytes(&self, object_key: &str) -> anyhow::Result<Vec<u8>>;
    pub async fn put_bytes(&self, object_key: &str, bytes: &[u8], content_type: &str) -> anyhow::Result<()>;
    pub fn sign_manifest(&self, manifest_json: &[u8]) -> anyhow::Result<Vec<u8>>;   // 64 字节
    pub fn verify_manifest(&self, manifest_json: &[u8], signature: &[u8]) -> bool;
}
```

- SigV4 query 预签名（GET/PUT），`path_style=true` 时 `{endpoint}/{bucket}/{key}`，否则虚拟主机；host 参与签名。
- 单测：用 AWS 官方 SigV4 query-presign 示例验证签名（固定日期注入），以及 sign/verify manifest 往返。

### 14.2 领域层 `qe_domain::storage`（`crates/domain/src/storage.rs`，加 `pub mod storage;`）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ImageKind { Docker, Oci, LxcRootfs, VmDisk }
#[derive(...)] #[serde(rename_all = "snake_case")]
pub enum ImageFormat { DockerArchive, OciLayout, Tar, Qcow2, Raw }
#[derive(...)] #[serde(rename_all = "lowercase")]
pub enum Compression { None, Zstd, Gzip }
#[derive(...)] #[serde(rename_all = "snake_case")]
pub enum ArtifactStatus { Uploaded, ManifestReady, Verifying, Verified, Published, Deprecated, Revoked }
#[derive(...)] #[serde(rename_all = "snake_case")]
pub enum TransferDirection { Download, Upload }
#[derive(...)] #[serde(rename_all = "snake_case")]
pub enum TransferStatus { Pending, Running, Completed, Failed, Expired }
#[derive(...)] #[serde(rename_all = "snake_case")]
pub enum CollectionStatus { Pending, Uploaded, Verified, Rejected }
#[derive(...)] #[serde(rename_all = "lowercase")]
pub enum ImageChannel { Dev, Test, Prod }

pub struct ManifestChunk { pub index: i32, pub object_key: String, pub sha256: String, pub size: i64, pub offset: i64 }
pub struct ManifestRuntime { pub docker_image_ref: Option<String>, pub lxc_distro: Option<String>,
    pub lxc_release: Option<String>, pub vm_os: Option<String>, pub vm_disk_size: Option<i64> }
pub struct Manifest { pub schema: i32, pub kind: ImageKind, pub name: String, pub version: String,
    pub platform: String, pub format: ImageFormat, pub compression: Compression,
    pub total_size: i64, pub chunks: Vec<ManifestChunk>, pub runtime: ManifestRuntime,
    pub annotations: serde_json::Value, pub created_at: DateTime<Utc>, pub created_by: Option<Uuid> }
impl Manifest {
    pub fn to_json(&self) -> DomainResult<Vec<u8>>;       // serde_json::to_vec（字段顺序固定）
    pub fn digest_hex(&self) -> DomainResult<String>;    // sha256(manifest_json) hex
}

pub struct StorageObject { id, bucket, object_key, sha256, size: i64, content_type, created_at }
pub struct ImageArtifact { id, name, version, kind, format, compression, platform,
    manifest_digest: Option<String>, total_size: i64, status: ArtifactStatus,
    created_by: Option<Uuid>, created_at, updated_at }
pub struct ImageChunk { manifest_digest, index: i32, object_key, sha256, size: i64, offset: i64 }
pub struct ImageVerification { id, artifact_id, ok: bool, report: serde_json::Value,
    verified_by: Uuid, verified_at: DateTime<Utc> }
pub struct ImageChannelRow { id, artifact_id, channel: ImageChannel, is_current: bool, created_at }
pub struct ImageAssignment { id, image_id, area_id: Option<Uuid>, room_id: Option<Uuid>,
    channel: ImageChannel, rollout_state: String, assigned_by: Uuid, assigned_at, updated_at }
pub struct Transfer { id, direction, room_id, image_id: Option<Uuid>, manifest_digest: Option<String>,
    status: TransferStatus, progress: i32, error: Option<String>, created_at, updated_at }
pub struct Collection { id, area_id: Option<Uuid>, room_id: Option<Uuid>, kind: String,
    object_key: String, sha256: String, size: i64, status: CollectionStatus, created_at }
pub struct NewImage { name, version, kind, format, compression, platform, created_by: Option<Uuid> }
pub struct ImageFilter { pub kind: Option<ImageKind>, pub status: Option<ArtifactStatus>, pub name: Option<String> }
```

校验（`crates/domain/src/validation.rs` 或 storage.rs 内）：
`validate_image_name`（1..=64，小写字母数字 `-_.`）、`validate_image_version`（1..=64，字母数字 `.-_+`）、`validate_platform`（`{os}/{arch}`，各 1..=32，字母数字 `_-.`）、`validate_sha256_hex`（64 hex 小写）、`validate_object_key`（≤512，无 `..`，仅字母数字 `-_.` 与 `/`）、`validate_manifest(&Manifest)`（schema=1；chunks 非空且 index 从 0 连续、offset 从 0 连续、size>0、total_size=sum；kind/format 组合合法：Docker→DockerArchive、Oci→OciLayout、LxcRootfs→Tar、VmDisk→Qcow2/Raw；compression 必须 None（P0）；同 chunk sha256 去重）。单元测试覆盖边界。

权限（`rbac.rs`）：`STORAGE_READ="storage:read"`、`STORAGE_MANAGE="storage:manage"`、`IMAGE_READ="image:read"`、`IMAGE_MANAGE="image:manage"`，加入 `ALL`。

### 14.3 迁移 `migrations/0011_storage.sql`

```sql
CREATE TABLE storage_objects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bucket TEXT NOT NULL,
    object_key TEXT NOT NULL UNIQUE,
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    size BIGINT NOT NULL CHECK (size >= 0),
    content_type TEXT NOT NULL DEFAULT 'application/octet-stream',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX storage_objects_sha256_idx ON storage_objects (sha256);

CREATE TABLE image_artifacts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    kind TEXT NOT NULL,
    format TEXT NOT NULL,
    compression TEXT NOT NULL DEFAULT 'none',
    platform TEXT NOT NULL,
    manifest_digest TEXT,
    total_size BIGINT NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'uploaded',
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (name, version, platform)
);
CREATE INDEX image_artifacts_status_idx ON image_artifacts (status, created_at DESC);

CREATE TABLE image_chunks (
    manifest_digest TEXT NOT NULL,
    index INT NOT NULL,
    object_key TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    size BIGINT NOT NULL,
    offset BIGINT NOT NULL,
    PRIMARY KEY (manifest_digest, index)
);

CREATE TABLE image_verifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_id UUID NOT NULL REFERENCES image_artifacts(id) ON DELETE CASCADE,
    ok BOOLEAN NOT NULL,
    report JSONB NOT NULL DEFAULT '{}'::jsonb,
    verified_by UUID REFERENCES users(id) ON DELETE SET NULL,
    verified_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX image_verifications_artifact_idx ON image_verifications (artifact_id, verified_at DESC);

CREATE TABLE image_channels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_id UUID NOT NULL REFERENCES image_artifacts(id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    is_current BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (artifact_id, channel)
);

CREATE TABLE image_assignments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    image_id UUID NOT NULL REFERENCES image_artifacts(id) ON DELETE CASCADE,
    area_id UUID REFERENCES areas(id) ON DELETE CASCADE,
    room_id UUID REFERENCES rooms(id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    rollout_state TEXT NOT NULL DEFAULT 'pending',
    assigned_by UUID REFERENCES users(id) ON DELETE SET NULL,
    assigned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (room_id, image_id)
);

CREATE TABLE transfers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    direction TEXT NOT NULL,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    image_id UUID REFERENCES image_artifacts(id) ON DELETE SET NULL,
    manifest_digest TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    progress INT NOT NULL DEFAULT 0,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX transfers_room_idx ON transfers (room_id, created_at DESC);

CREATE TABLE collections (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    area_id UUID REFERENCES areas(id) ON DELETE SET NULL,
    room_id UUID REFERENCES rooms(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    object_key TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    size BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 权限
INSERT INTO permissions (code, name, description) VALUES
 ('storage:read','查看存储资源','查看存储对象、分发/收集任务'),
 ('storage:manage','管理存储资源','上传注册对象、管理分发/收集任务'),
 ('image:read','查看镜像','查看镜像、清单与验证报告'),
 ('image:manage','管理镜像','创建/验证/发布/分配镜像')
ON CONFLICT (code) DO NOTHING;
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p WHERE r.code='admin' ON CONFLICT DO NOTHING;
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r JOIN permissions p ON p.code IN ('storage:read','image:read')
WHERE r.code IN ('viewer','editor') ON CONFLICT DO NOTHING;
```

### 14.4 仓储 `crates/db/src/storage.rs`（`pub mod storage;`）

```rust
pub struct StorageRepo { /* PgPool */ }
impl StorageRepo {
    pub fn new(db: PgPool) -> Self;
    pub async fn upsert_object(&self, bucket:&str, object_key:&str, sha256:&str, size:i64, content_type:&str) -> DbResult<StorageObject>;
    pub async fn get_object(&self, sha256:&str) -> DbResult<Option<StorageObject>>;
    pub async fn create_image(&self, input:&NewImage) -> DbResult<ImageArtifact>;
    pub async fn list_images(&self, filter:&ImageFilter, page:i64, page_size:i64) -> DbResult<Page<ImageArtifact>>;
    pub async fn get_image(&self, id:Uuid) -> DbResult<Option<ImageArtifact>>;
    pub async fn mark_manifest_ready(&self, id:Uuid, manifest_digest:&str, total_size:i64) -> DbResult<Option<ImageArtifact>>;
    pub async fn set_status(&self, id:Uuid, status:ArtifactStatus) -> DbResult<Option<ImageArtifact>>;
    pub async fn replace_chunks(&self, manifest_digest:&str, chunks:&[ImageChunk]) -> DbResult<()>;
    pub async fn chunks(&self, manifest_digest:&str) -> DbResult<Vec<ImageChunk>>;
    pub async fn insert_verification(&self, artifact_id:Uuid, ok:bool, report:serde_json::Value, verified_by:Uuid) -> DbResult<ImageVerification>;
    pub async fn latest_verification(&self, artifact_id:Uuid) -> DbResult<Option<ImageVerification>>;
    pub async fn publish(&self, id:Uuid, channel:ImageChannel) -> DbResult<Option<ImageArtifact>>;
    pub async fn channels(&self, id:Uuid) -> DbResult<Vec<ImageChannelRow>>;
    pub async fn assign_room(&self, image_id:Uuid, room_id:Uuid, channel:ImageChannel, actor:Uuid) -> DbResult<Option<ImageAssignment>>;
    pub async fn unassign_room(&self, image_id:Uuid, room_id:Uuid) -> DbResult<bool>;
    pub async fn list_transfers(&self, room_id:Option<Uuid>, page:i64, page_size:i64) -> DbResult<Page<Transfer>>;
    pub async fn get_transfer(&self, id:Uuid) -> DbResult<Option<Transfer>>;
    pub async fn retry_transfer(&self, id:Uuid) -> DbResult<bool>;
    pub async fn list_collections(&self, room_id:Option<Uuid>, page:i64, page_size:i64) -> DbResult<Page<Collection>>;
}
```

实现要点：`replace_chunks` 事务（DELETE + INSERT）；`publish` 事务（要求 status=verified，更新 status=published + upsert channel current）；`assign_room` 事务（校验 image 存在且已 published、room 存在 → upsert assignment、插入一条 `transfers(direction=download,status=pending)`）；`unassign_room` 删除 assignment；`retry_transfer` 把 failed/expired 重置为 pending（progress=0,error=null）。

### 14.5 API（`crates/api/src/api/admin/storage.rs` + `images.rs`）

`AppState` 增加 `pub storage: Option<std::sync::Arc<qe_storage::StorageService>>`；`AppState::new` 中由 `config.storage` 构建，失败只记 error 并置 None；所有存储/镜像接口在 None 时返回 `503 service_unavailable`（"未配置 S3"）。Config 增加：

```rust
pub struct StorageConfig { pub endpoint: String, pub region: String, pub bucket: String,
    pub access_key: String, pub secret_key: String, pub path_style: bool,
    pub presign_ttl_secs: u64, pub signing_key_base64: Option<String> }
pub struct Config { /* ... */ pub storage: Option<StorageConfig> }
```

环境变量：`S3_ENDPOINT`、`S3_REGION`（默认 us-east-1）、`S3_BUCKET`、`S3_ACCESS_KEY`、`S3_SECRET_KEY`、`S3_PATH_STYLE`（默认 true）、`STORAGE_PRESIGN_TTL_SECONDS`（默认 900，60..=86400）、`STORAGE_SIGNING_KEY_BASE64`（可选）。设置 `S3_ENDPOINT` + `S3_BUCKET` 即启用，否则 `storage=None`。

接口（全部 `/api/v1/admin`）：

| 方法 | 路径 | 权限 | 说明 |
| --- | --- | --- | --- |
| POST | `/storage/objects/presign` | `storage:manage` | body `{sha256,size,content_type?}` → 校验 sha256/size，返回 `{object_key,upload_url,expires_at}`（object_key=`objects/{sha256}`） |
| POST | `/storage/objects` | `storage:manage` | body `{sha256,size,content_type?}`；服务端从 S3 读取、校验 sha256+size，成功落库；失败 400/503 |
| GET | `/storage/objects/{sha256}` | `storage:read` | 对象详情 + 预签名下载 URL |
| POST | `/images` | `image:manage` | body `{name,version,kind,format,compression,platform}` |
| GET | `/images?kind=&status=&name=&page=&page_size=` | `image:read` | 分页 |
| GET | `/images/{id}` | `image:read` | 详情 + chunks + 最近验证 + channels |
| POST | `/images/{id}/manifest` | `image:manage` | body `{total_size,parts:[{index,sha256,size,offset}],runtime:{...},annotations?}`；校验对象已注册、size 匹配；生成 manifest→签名→写 S3（`manifests/{digest}.json`、`signatures/{digest}.sig`）→ 落 chunks → status=manifest_ready |
| POST | `/images/{id}/verify` | `image:manage` | 同步验证：逐 chunk 从 S3 读取并校验 sha256/size、总大小、格式 magic、manifest 签名；写 verification、status=verified 或 failed |
| POST | `/images/{id}/publish` | `image:manage` | body `{channel:"dev"\|"test"\|"prod"}`；仅 verified 可发布 |
| GET | `/images/{id}/manifest` | `image:read` | 返回 manifest JSON + 预签名 raw 下载 URL |
| PUT | `/rooms/{id}/images/{image_id}` | `image:manage` | body `{channel?}`；分配并创建 transfer |
| DELETE | `/rooms/{id}/images/{image_id}` | `image:manage` | 取消分配 |
| GET | `/transfers?room_id=&page=&page_size=` | `storage:read` | 分发/收集任务 |
| POST | `/transfers/{id}/retry` | `storage:manage` | 重试 |
| GET | `/collections?room_id=&page=&page_size=` | `storage:read` | 收集列表 |

- 审计：`storage.object.register`、`image.create/verify/publish/assign/unassign`、`transfer.retry`。
- 验证报告 report 结构：`{ "manifest_digest": "...", "checks": [{"name":"chunk.0.sha256","ok":true,"detail":"..."}, ...], "errors": [] }`。
- 格式 magic（compression=none）：qcow2 `QFI\xfb`；tar/docker-archive/oci-layout 在 offset 257 有 `ustar`；raw 跳过。
- `manifest` 响应字段与 §4 一致（不含 signature；signature 单独存 `.sig`）。
- OpenAPI 增加 `Storage`/`Images` tag；README 增加「资源与镜像管理（P0）」章节。

### 14.6 测试与验收（Lead 负责）

- `make check` 通过；`0011` 在 `qestudio_test` 应用成功。
- 新增 `crates/api/tests/storage.rs`（`#[ignore]`）：
  - 进程内 fake S3（axum，忽略签名，PUT/GET 内存 map）；`StorageConfig` 指向它；
  - 管理员登录 → 创建 image → presign/上传 2 个 chunk（qcow2 magic + 填充）→ 注册对象 → 提交 manifest → verify 通过 → publish → 分配房间（产生 transfer）→ 查询任务/详情 → 预签名 manifest 下载；
  - 负例：sha256 不匹配 400、未验证发布 409、viewer 403。
- `scripts/test-integration.sh` 增加 storage 测试；README/OpenAPI 一致性检查。
- RustFS 部署配置写入 `compose.yaml`（可选 profile `storage`），本地测试不依赖真实 RustFS。

---

## 15. P1：房间节点 agent（复用 device-pki 认证与消息体系）

> 原则：**不新增认证/长连接体系**。节点 agent 是 device-pki 的一个“业务客户端”：
> 复用 `qe-device-pki-client` 的注册、mTLS、WebSocket 隧道/下行拉取与上行发送；
> 大文件仍走 S3 预签名直连，device-pki 消息只承载控制信令与状态（≤64KB）。

### 15.1 组成

- 新 crate `crates/node-agent`（包名 `qe-node-agent`，独立二进制），依赖 `qe-device-pki-client`。
- device-pki 服务端（`crates/device-pki`）扩展：
  - 新增业务消息 kind（协议文档同步更新 `docs/device-pki-protocol.md`）；
  - mTLS listener 新增资源预签名端点（见 §15.3），只对已激活设备开放。

### 15.2 消息协议（device-pki kind）

下行（控制面 → 节点）：

```json
{ "kind": "image.pull", "payload": {
    "transfer_id": "uuid", "image_id": "uuid",
    "name": "win11-base", "version": "2026.10.31-1",
    "type": "vm_disk", "format": "qcow2", "platform": "linux/amd64",
    "manifest_digest": "<sha256>", "total_size": 32212254720, "chunk_count": 2000,
    "presign_endpoint": "/resources/presign" } }
```

上行（节点 → 控制面）：

```json
{ "kind": "transfer.progress", "payload": { "transfer_id": "uuid", "phase": "download|verify|import",
    "progress": 42, "bytes_done": 123456, "error": null } }
{ "kind": "transfer.completed", "payload": { "transfer_id": "uuid", "ok": true,
    "manifest_digest": "<sha256>", "verified": true, "imported_ref": "docker:win11-base:…", "error": null } }
```

- device-pki 服务端收到上行消息后，按 kind 注册的 handler 更新 `transfers`（`pending→running→completed/failed`）。
- 复用现有上行收件箱去重、下行 ACK 与离线判定；不新增可靠传输机制。

### 15.3 预签名获取（推荐方案 A）

agent 在 mTLS 通道内请求短 TTL 预签名 URL，不持有 S3 凭证，也不使用用户 Bearer：

```
POST /resources/presign          (mTLS listener，证书 identity = device slot)
body: { "manifest_digest": "<sha256>", "kind": "manifest"|"chunk"|"signature",
        "index": 0, "count": 64 }
resp: { "urls": [{ "key": "objects/<sha>", "url": "https://…", "expires_at": "…" }],
        "public_key_base64": "<Ed25519 公钥>" }
```

- device-pki 服务端引入 `qe-storage` + 与 `qe-api` **共用 S3/签名配置**（同一套 env），只做“验签 + 生成预签名 URL”，不代理字节。
- 授权：证书 slot 必须已绑定到房间（`rooms.device_slot`），且该 manifest 已分配到该房间（`image_assignments`）或房间所属区域。
- 备选方案 B：主 API 发短期设备 token（HMAC），agent 用它换预签名 URL；需要新增 token 表/轮换，不如 A 复用 mTLS 干净。

### 15.4 agent 流程

1. 长连接收到 `image.pull`（或断线后从 offline 兜底轮询拿到）。
2. mTLS 预签名拉 `manifests/{digest}.json` → 用 `public_key_base64` 验签 → 逐 chunk 拉取（并发/断点/本地缓存，`objects/{sha256}` 只补缺失）。
3. 校验：chunk sha256/size、总大小、格式 magic；失败上报 `transfer.completed{ok:false}` 并保留上一版本。
4. 导入（按 `type` 选择适配器）：
   - `docker` / `oci`：`docker load -i` 或 `ctr images import`；
   - `lxc_rootfs`：**Incus 优先** `incus image import`；plain LXC 解包到 `/var/lib/lxc/<name>/rootfs`；
   - `vm_disk`（qcow2/raw）：通过 `virsh vol-upload`（或直接落盘）进入 libvirt storage pool，登记卷名。
5. 上报告 `transfer.progress` / `transfer.completed`；成功后记录本地版本，供回滚使用。
6. 本地缓存目录按 `CACHE_MAX_BYTES` LRU 淘汰，但保留当前版本。

### 15.5 配置

agent：

| 环境变量 | 说明 |
| --- | --- |
| `DEVICE_PKI_MTLS_URL` | device-pki mTLS 地址（默认 `https://127.0.0.1:4441`） |
| `DEVICE_PKI_CERT` / `DEVICE_PKI_KEY` / `DEVICE_PKI_CA` | 设备证书与 CA（复用现有客户端凭证） |
| `CACHE_DIR` / `CACHE_MAX_BYTES` | chunk/manifest 缓存 |
| `DOWNLOAD_CONCURRENCY` | 并发分片数（默认 4） |
| `DOCKER_BIN` / `CTR_BIN` / `INCUS_BIN` / `VIRSH_BIN` | 导入命令路径（便于测试 mock） |

device-pki 服务端：复用 `S3_ENDPOINT/S3_BUCKET/S3_*` 与 `STORAGE_SIGNING_KEY_BASE64`（公钥下发；私钥仍只在本机，不对设备暴露）。

### 15.6 验收

- 单元测试：manifest 验签/校验、预签名请求构造、导入命令构造（mock 可执行文件）、缓存 LRU。
- 集成测试：fake S3 + device-pki mTLS（复用现有 enroll_flow 测试设施）+ fake `docker`/`virsh` 脚本，覆盖 assign→downlink→pull→verify→import→completed→`transfers` 状态变化。
- 手工：真实 RustFS/Incus 环境跑通一个 rootfs 与一个 qcow2 的端到端。

---

## 16. P1 实现契约（v1，开工版）

> 决策：预签名走 device-pki mTLS 端点；首批同时实现 Docker/OCI、LXC/Incus、VM/libvirt 三个导入适配器；
> agent 为独立 crate `qe-node-agent`，依赖 `qe-device-pki-client`，不新增认证体系。**大文件不走 device-pki 消息**。

### 16.1 控制面消息（device-pki kind）

下行 `image.pull`（qe-api 在房间分配镜像后投递到 `device_downlink`）：

```json
{ "transfer_id": "uuid", "image_id": "uuid", "name": "...", "version": "...",
  "kind": "docker|oci|lxc_rootfs|vm_disk", "format": "docker_archive|oci_layout|tar|qcow2|raw",
  "platform": "linux/amd64", "manifest_digest": "<64hex>",
  "total_size": 123, "chunk_count": 2, "presign_endpoint": "/resources/presign" }
```

上行（agent → device-pki `/messages`，`payload` 为 JSON 字符串）：

```json
{ "kind": "transfer.progress", "payload": { "transfer_id": "uuid", "phase": "download|verify|import",
    "progress": 0..100, "bytes_done": 123, "error": null } }
{ "kind": "transfer.completed", "payload": { "transfer_id": "uuid", "ok": true,
    "manifest_digest": "<64hex>", "verified": true, "imported_ref": "docker:name:version", "error": null } }
```

device-pki 服务端注册两个 handler：
- `transfer.progress`：`UPDATE transfers SET status='running', progress=$2, error=NULL, updated_at=now() WHERE id=$1`；
- `transfer.completed`：`ok=true` → `status='completed', progress=100`，并 `image_assignments.rollout_state='deployed'`；
  `ok=false` → `status='failed', error=$err`，`image_assignments.rollout_state='failed'`。

### 16.2 device-pki：`POST /resources/presign`（mTLS listener）

请求（`PeerCert` 提供 serial；同现有 mTLS handler）：

```json
{ "manifest_digest": "<64hex>", "kind": "manifest"|"signature"|"chunk",
  "index": 0, "count": 64 }
```

响应：

```json
{ "urls": [ { "key": "objects/<sha>", "url": "https://…", "expires_at": "RFC3339" } ],
  "public_key_base64": "…" }
```

- 授权（fail-closed）：
  1. serial → `device_enrollment.device_slot`（取 `status IN ('issued','active')`）→ `rooms.device_slot` → `room_id`；
  2. `image_chunks.manifest_digest` 对应的 `image_artifacts.manifest_digest` 必须存在，且
     `image_assignments` 中存在 `(image_id, room_id)`；
  3. 不满足 → 403；对象不存在 → 404。
- `kind=manifest` 返回 `manifests/{digest}.json`；`signature` 返回 `signatures/{digest}.sig`；
  `chunk` 按 `index` 顺序返回最多 `count`（clamp 1..=64）个 `objects/{chunk.sha256}`。
- 服务端新增 `AppState.storage: Option<Arc<qe_storage::StorageService>>`；main 从 `S3_*` 构建（与 qe-api 同套 env），
  未配置时该端点返回 503；`ExpiresAt` 用 `STORAGE_PRESIGN_TTL_SECONDS`（默认 900）。
- `crates/device-pki/Cargo.toml` 增加 `qe-storage` 依赖。

### 16.3 qe-api/qe-db：分配后投递 `image.pull`

- `StorageRepo::assign_room` 返回值改为：

```rust
pub struct AssignOutcome { pub assignment: ImageAssignment, pub transfer: Transfer }
pub async fn assign_room(&self, image_id: Uuid, room_id: Uuid, channel: ImageChannel, actor: Uuid)
    -> DbResult<Option<AssignOutcome>>;
pub async fn chunk_count(&self, manifest_digest: &str) -> DbResult<i64>;
/// 与 schedule 的 downlink 语义一致：按 device_slot 找存活证书，写 device_downlink，返回 serial。
pub async fn enqueue_downlink_for_slot(&self, slot: &str, kind: &str, payload: &str) -> DbResult<Option<String>>;
```

- `PUT /rooms/{id}/images/{image_id}` 成功后：若房间 `device_slot` 非空且有存活设备，构造 §16.1 的 payload 调
  `enqueue_downlink_for_slot(slot, "image.pull", payload)`，并 `state.cache.publish(state.config.schedule_downlink_notify_channel, serial)`；
  无槽位/无设备只记 `warn!`，transfer 保持 pending。响应仍为 assignment。
- 集成测试扩展：房间绑定设备 + 插入 `device_enrollment/device_session` 后分配镜像，断言 `device_downlink.kind='image.pull'`。

### 16.4 node-agent crate（`crates/node-agent`，包名 `qe-node-agent`）

目录（Lead 会建脚手架与 Cargo.toml）：

```text
crates/node-agent/
  Cargo.toml
  src/main.rs          # 读 env、确认证书、订阅 image.pull、轮询下行
  src/lib.rs           # pub mod ...
  src/config.rs        # AgentConfig::from_env
  src/manifest.rs      # 与 §14.2 对齐的 Manifest/Chunk 结构 + sha256 digest + Ed25519 验签
  src/presign.rs       # PresignClient trait + mTLS 实现（POST /resources/presign）
  src/cache.rs         # 内容寻址缓存 CACHE_DIR/objects/{sha256}，LRU 到 CACHE_MAX_BYTES
  src/pull.rs          # 拉 manifest/签名/chunk、校验、组装，阶段回调 progress
  src/report.rs        # 通过 qe-device-pki-client 上行 transfer.progress/completed（OutboxQueue）
  src/importer/mod.rs  # TopologyImporter trait + dispatch（按 kind/format）
  src/importer/docker.rs
  src/importer/incus.rs   # 同时兼容 plain LXC（rootfs 解包）
  src/importer/vm.rs      # qcow2/raw → libvirt storage pool
  src/job.rs           # 一次 image.pull 的编排：pull → verify → import → report
```

关键签名（db/api/agent 三方契约）：

```rust
pub struct AgentConfig {
    pub mtls_endpoint: String,
    pub slot: String,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub cache_max_bytes: u64,
    pub download_concurrency: usize,
    pub docker_bin: String,   // 默认 docker
    pub ctr_bin: String,      // 默认 ctr
    pub incus_bin: String,    // 默认 incus
    pub virsh_bin: String,    // 默认 virsh
    pub lxc_rootfs_dir: PathBuf, // 默认 /var/lib/lxc
}
impl AgentConfig { pub fn from_env() -> anyhow::Result<Self>; }

pub trait PresignClient: Send + Sync {
    fn presign<'a>(&'a self, request: &'a PresignRequest)
        -> BoxFuture<'a, anyhow::Result<PresignReply>>;
}

pub struct ImportRequest {
    pub kind: String, pub format: String, pub name: String, pub version: String,
    pub staging_dir: PathBuf,      // 校验后落盘目录（含组装文件/tar）
    pub artifact_path: PathBuf,    // 单个文件（qcow2/tar/rootfs tar）或 OCI 目录
    pub imported_name: String,     // 导入后的本地名称/引用
}
pub struct ImportResult { pub imported_ref: String }
pub trait TopologyImporter: Send + Sync {
    fn supports(&self, kind: &str, format: &str) -> bool;
    fn import<'a>(&'a self, request: &'a ImportRequest) -> BoxFuture<'a, anyhow::Result<ImportResult>>;
}
```

- 导入实现（命令路径可配，便于测试 mock）：
  - Docker `docker_archive`：`{docker_bin} load -i <artifact_path>`；`oci_layout`：`{ctr_bin} images import <artifact_path>`；
  - Incus `tar`：`{incus_bin} image import <artifact_path> alias=<imported_name>`；若 `incus_bin` 为空/不存在则 plain LXC 兜底：
    解包 tar 到 `{lxc_rootfs_dir}/<name>/rootfs`（先 staging 解包再原子 rename）；
  - VM `qcow2`/`raw`：`{virsh_bin} vol-create-as <pool> <vol> <size>` + `vol-upload <vol> <artifact_path> --pool <pool>`；
    pool 名从 env `LIBVIRT_POOL`（默认 `default`），卷名 `<name>-<version>`。
- 拉取校验：manifest 签名（Ed25519，公钥来自 presign 响应）、chunk sha256/size、总大小、格式 magic
  （qcow2 `QFI\xfb`；tar 在 offset 257 `ustar`；raw 跳过）。
- 上报：progress 阶段（download 0-80、verify 80-90、import 90-100）；completed 带 `imported_ref`。
- main 循环：`confirm`（已激活则跳过）→ 注册 `image.pull` handler → `downlink::poll_once` 周期 2s；handler 内串行执行 job（避免并发导入）。
- 单测（不依赖真实设备/网络）：
  - manifest 验签/伪签名拒绝、chunk 校验、cache LRU、config 解析；
  - 用 `PresignClient` mock + 内存 S3/文件 URL 或 trait mock 测 pull；
  - importer 用临时 fake 可执行脚本（`tempfile`）断言命令参数与 staging 布局；plain LXC 解包测试用临时目录。

### 16.5 配置（agent）

| 变量 | 默认 | 说明 |
| --- | --- | --- |
| `DEVICE_PKI_MTLS_URL` | 无（必填） | 如 `https://node:4441` |
| `DEVICE_SLOT` | 无（必填） | 设备槽位（与证书 CN 一致） |
| `DEVICE_DATA_DIR` | `./data` | 客户端状态/凭证目录（沿用 device-pki-client） |
| `CACHE_DIR` | `./cache` | 分片缓存 |
| `CACHE_MAX_BYTES` | `10737418240` | 缓存上限 |
| `DOWNLOAD_CONCURRENCY` | `4` | 并发分片 |
| `DOCKER_BIN` / `CTR_BIN` / `INCUS_BIN` / `VIRSH_BIN` | 对应命令名 | 可指向 mock 脚本 |
| `LIBVIRT_POOL` | `default` | libvirt storage pool |
| `LXC_ROOTFS_DIR` | `/var/lib/lxc` | plain LXC 根目录 |

### 16.6 验收

- `make check` 通过（新 crate 加入 workspace；agent/device-pki 单测通过）。
- `scripts/test-integration.sh` 全绿；storage 集成测试扩展断言分配后 `device_downlink` 出现 `image.pull`。
- device-pki：`/resources/presign` 路由可编译 + 授权/取键逻辑单测；真实 mTLS 端到端由后续手工/环境测试。
- node-agent：单元测试覆盖 manifest 校验、cache、job 编排（mock PresignClient/TopologyImporter）、三个 importer 命令构造。
