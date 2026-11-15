// 房间启动配置（startup provisioning）的前端类型。
//
// 契约见 room_node/docs/startup-provisioning.md §2/§3。这些类型刻意不放进
// `schema.ts`：该文件由 `docs/openapi.json` 生成（`pnpm run types` 会整体重写），
// 而 startup 的 `network` 字段目前只在文档/后端契约里，重新生成会丢掉手写类型。
import type { ArtifactStatus, ImageKind } from './schema'

export type RoomStartupNetworkMode = 'isolated' | 'nat' | 'none'

/** allowlist 端口：数字单端口或 `"8000-8100"` 范围字符串。 */
export type RoomStartupPort = number | string

export interface RoomStartupAllowService {
  cidr: string
  proto?: 'tcp' | 'udp' | 'any' | null
  ports: RoomStartupPort[]
}

/** 每台 VM 的配网声明；`null` / 缺省 = 不配网（旧配置语义）。 */
export interface RoomStartupVmNetwork {
  mode: RoomStartupNetworkMode
  vlan_id?: number | null
  subnet?: string | null
  allow_services?: RoomStartupAllowService[]
  allow_internet?: boolean
  allow_same_room?: boolean
}

export interface RoomStartupImageRef {
  image_id: string
}

export interface RoomStartupVm {
  name: string
  vcpus: number
  memory_mib: number
  disk_gb: number
  base_volume?: string | null
  base_path?: string | null
  gpu?: string[]
  auto_start: boolean
  network?: RoomStartupVmNetwork | null
}

export interface RoomStartupConfig {
  auto_pull: boolean
  auto_start_vm: boolean
  images: RoomStartupImageRef[]
  vms: RoomStartupVm[]
  /**
   * fallback 镜像开关：启用后 agent 把该镜像长期缓存在设备本地，
   * 控制面不可达时用本地缓存兜底（见 docs/fallback-image.md）。
   * 旧配置缺省 false。
   */
  fallback_enabled: boolean
  /** fallback 镜像 id；`fallback_enabled=true` 时必填。 */
  fallback_image_id?: string | null
}

/** 管理端读取启动配置时回查出的镜像信息；`assigned` 表示已分配到该房间（预签名授权前提）。 */
export interface RoomStartupImageInfo {
  image_id: string
  name: string
  version: string
  kind: ImageKind
  status: ArtifactStatus
  manifest_digest?: string | null
  total_size: number
  transfer_id?: string | null
  assigned: boolean
}

/** 启动配置生效目标：房间主设备槽位 + 子机槽位。 */
export interface RoomStartupTarget {
  device_slot: string
  serial: string | null
}

export interface RoomStartupProfile {
  room_id: string
  enabled: boolean
  revision: number
  config: RoomStartupConfig
  updated_at?: string | null
  images: RoomStartupImageInfo[]
  /** 配置里选中的 fallback 镜像（未选择 / 镜像已删除时为 null）。 */
  fallback_image?: RoomStartupImageInfo | null
  targets: RoomStartupTarget[]
}

export interface RoomStartupSaveRequest {
  enabled: boolean
  config: RoomStartupConfig
}
