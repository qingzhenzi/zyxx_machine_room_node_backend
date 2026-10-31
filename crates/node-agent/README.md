# qe-node-agent（房间节点 agent，P1）

经 **device-pki mTLS** 拉取、校验并导入镜像/资源；大文件走 S3 预签名直连，
device-pki 消息只承载控制信令与状态。设计契约见
[`docs/storage-design.md`](../../docs/storage-design.md) §15–§16。

## 流程

1. 复用 `qe-device-pki-client` 的证书/TOFU 信任与 mTLS 传输（不新增认证体系）；
2. 下行 `image.pull`（device-pki kind）触发任务；
3. 通过 mTLS `POST /resources/presign` 获取 manifest / 分片 / 签名的短 TTL 预签名 URL；
4. 拉取 → Ed25519 验签 manifest → 逐 chunk 校验 sha256/size → 格式 magic；
5. 按类型导入：
   - `docker_archive` → `docker load -i`；`oci_layout` → `ctr images import`
   - `tar`（LXC）→ 合成 metadata 后 `incus image import`；`LXC_AUTO_START=true` 时继续 `incus launch`；
     无 Incus 时 plain LXC 解包到 `LXC_ROOTFS_DIR/<name>/rootfs`
   - `qcow2`/`raw`（虚拟机）→ `virsh vol-create-as` + `vol-upload` 到 `LIBVIRT_POOL`
6. 上行 `transfer.progress` / `transfer.completed` 回写 `transfers` 与房间发布状态。

## 配置

| 变量 | 默认 | 说明 |
| --- | --- | --- |
| `DEVICE_PKI_MTLS_URL` | 必填 | 如 `https://node:4441` |
| `DEVICE_SLOT` | 必填 | 设备槽位（与证书 CN 一致） |
| `DEVICE_DATA_DIR` | `./data` | 客户端状态/凭证目录（复用 device-pki-client） |
| `CACHE_DIR` | `./cache` | 内容寻址分片缓存 |
| `CACHE_MAX_BYTES` | `10737418240` | 缓存上限（LRU） |
| `DOWNLOAD_CONCURRENCY` | `4` | 并发分片数 |
| `DOCKER_BIN` / `CTR_BIN` / `INCUS_BIN` / `VIRSH_BIN` | 同名命令 | 可指向 mock 脚本 |
| `LIBVIRT_URI` | `qemu:///system` | libvirt 连接 URI（非 root 必须显式，否则默认 session 连接找不到 pool） |
| `LIBVIRT_POOL` | `default` | libvirt storage pool |
| `LXC_AUTO_START` | `false` | Incus 导入成功后是否自动 `incus launch` 启动实例 |
| `LXC_ROOTFS_DIR` | `/var/lib/lxc` | plain LXC rootfs 目录 |

## 运行

```bash
cargo build -p qe-node-agent
DEVICE_PKI_MTLS_URL=https://node:4441 DEVICE_SLOT=room-101 \
  DEVICE_DATA_DIR=/var/lib/qe-agent RUST_LOG=info \
  ./target/debug/qe-node-agent
```

> 生产环境必须为 device-pki 服务端与 qe-api 配置相同的 `STORAGE_SIGNING_KEY_BASE64`，
> 否则重启后 manifest 公钥变化，已签清单无法再验签。
