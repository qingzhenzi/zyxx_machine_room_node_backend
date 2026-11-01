import type { Permission } from '@/api/schema'

export interface PermissionGroup {
  key: string
  label: string
  permissions: Permission[]
}

/** 权限点按资源前缀分组，顺序尽量与后台导航一致。 */
const GROUP_LABELS: Record<string, string> = {
  announcement: '公告',
  user: '用户',
  role: '角色',
  audit: '审计',
  area: '区域',
  room: '房间',
  schedule: '课表与事件',
  device: '设备',
  storage: '存储',
  image: '镜像',
}

const GROUP_ORDER = [
  'announcement',
  'user',
  'role',
  'audit',
  'area',
  'room',
  'schedule',
  'device',
  'storage',
  'image',
]

export function permissionPrefix(code: string | undefined): string {
  if (!code) return 'other'
  const separator = code.indexOf(':')
  return separator > 0 ? code.slice(0, separator) : 'other'
}

export function groupPermissions(permissions: Permission[]): PermissionGroup[] {
  const groups = new Map<string, PermissionGroup>()
  for (const permission of permissions) {
    const key = permissionPrefix(permission.code)
    const group = groups.get(key)
    if (group) group.permissions.push(permission)
    else groups.set(key, { key, label: GROUP_LABELS[key] ?? '其他', permissions: [permission] })
  }

  const ordered: PermissionGroup[] = []
  for (const key of GROUP_ORDER) {
    const group = groups.get(key)
    if (group) {
      ordered.push(group)
      groups.delete(key)
    }
  }
  for (const group of groups.values()) ordered.push(group)
  return ordered
}

export function permissionName(permission: Permission): string {
  return permission.name || permission.code || '未命名权限'
}
