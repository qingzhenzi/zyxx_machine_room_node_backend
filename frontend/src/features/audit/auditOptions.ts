export interface SelectOption {
  value: string
  label: string
}

export interface ActionGroup {
  label: string
  actions: SelectOption[]
}

/** 后端记录的审计资源类型（`resource_type` 精确匹配）。 */
export const RESOURCE_TYPE_OPTIONS: SelectOption[] = [
  { value: 'user', label: '用户' },
  { value: 'role', label: '角色' },
  { value: 'announcement', label: '公告' },
  { value: 'area', label: '区域' },
  { value: 'room', label: '房间' },
  { value: 'device', label: '设备' },
  { value: 'schedule', label: '课表' },
  { value: 'schedule_event', label: '课表事件' },
  { value: 'image', label: '镜像' },
  { value: 'manifest', label: '镜像清单' },
  { value: 'storage_object', label: '存储对象' },
  { value: 'transfer', label: '分发任务' },
  { value: 'agent_config', label: '节点配置' },
  { value: 'config', label: '配置下发' },
]

/** 审计动作（`action` 精确匹配），按资源分组便于下拉选择。 */
export const ACTION_GROUPS: ActionGroup[] = [
  {
    label: '用户',
    actions: [
      { value: 'user.create', label: '创建用户' },
      { value: 'user.update', label: '更新用户' },
      { value: 'user.set_roles', label: '设置用户角色' },
    ],
  },
  {
    label: '角色',
    actions: [
      { value: 'role.create', label: '创建角色' },
      { value: 'role.set_permissions', label: '设置角色权限' },
      { value: 'role.delete', label: '删除角色' },
    ],
  },
  {
    label: '公告',
    actions: [
      { value: 'announcement.create', label: '创建公告' },
      { value: 'announcement.update', label: '更新公告' },
      { value: 'announcement.delete', label: '删除公告' },
    ],
  },
  {
    label: '区域 / 房间',
    actions: [
      { value: 'area.create', label: '创建区域' },
      { value: 'area.update', label: '更新区域' },
      { value: 'area.delete', label: '删除区域' },
      { value: 'room.create', label: '创建房间' },
      { value: 'room.update', label: '更新房间' },
      { value: 'room.delete', label: '删除房间' },
      { value: 'room.bind_device', label: '绑定设备' },
      { value: 'room.unbind_device', label: '解绑设备' },
      { value: 'room.issue_device_token', label: '生成设备注册 token' },
      { value: 'room.set_event_rules', label: '设置房间触发规则' },
    ],
  },
  {
    label: '课表 / 事件',
    actions: [
      { value: 'schedule.create', label: '创建课表' },
      { value: 'schedule.update', label: '更新课表' },
      { value: 'schedule.delete', label: '删除课表' },
      { value: 'schedule.set_weekly_slots', label: '保存周模板' },
      { value: 'schedule.set_exception', label: '设置单日覆盖' },
      { value: 'schedule.delete_exception', label: '删除单日覆盖' },
      { value: 'schedule_event.replay', label: '重放事件' },
      { value: 'schedule_event.retry', label: '重试投递' },
    ],
  },
  {
    label: '镜像 / 存储',
    actions: [
      { value: 'image.create', label: '创建镜像' },
      { value: 'image.verify', label: '校验镜像' },
      { value: 'image.publish', label: '发布镜像' },
      { value: 'image.assign', label: '分配镜像' },
      { value: 'image.unassign', label: '取消镜像分配' },
      { value: 'image.pull', label: '拉取镜像' },
      { value: 'manifest.digest', label: '写入镜像清单' },
      { value: 'manifest.signature', label: '签名镜像清单' },
      { value: 'storage.object.register', label: '登记存储对象' },
      { value: 'transfer.retry', label: '重试分发任务' },
    ],
  },
  {
    label: '节点 / 设备',
    actions: [
      { value: 'agent_config.update', label: '更新节点配置' },
      { value: 'config.update', label: '下发节点配置' },
      { value: 'device.presence', label: '设备在线状态变更' },
    ],
  },
]

const ACTION_LABELS = new Map<string, string>(
  ACTION_GROUPS.flatMap((group) =>
    group.actions.map((action) => [action.value, action.label] as const),
  ),
)

const RESOURCE_LABELS = new Map<string, string>(
  RESOURCE_TYPE_OPTIONS.map((option) => [option.value, option.label] as const),
)

export function actionLabel(action: string | undefined): string {
  if (!action) return '—'
  return ACTION_LABELS.get(action) ?? action
}

export function resourceLabel(resourceType: string | undefined): string {
  if (!resourceType) return '—'
  return RESOURCE_LABELS.get(resourceType) ?? resourceType
}
