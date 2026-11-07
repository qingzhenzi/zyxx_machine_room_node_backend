/**
 * 后端权限点常量与判断工具。权限点的事实来源是 `crates/domain/src/rbac.rs`，
 * 角色/权限可以在后台运行时修改，所以这里只用于隐藏明显不可用的入口，
 * 真正的授权仍由每个接口自行校验（403 由页面兜底提示）。
 */
export const PERMISSIONS = {
  announcementRead: 'announcement:read',
  announcementWrite: 'announcement:write',
  announcementPublish: 'announcement:publish',
  announcementDelete: 'announcement:delete',
  userManage: 'user:manage',
  roleManage: 'role:manage',
  auditRead: 'audit:read',
  areaRead: 'area:read',
  areaManage: 'area:manage',
  roomRead: 'room:read',
  roomManage: 'room:manage',
  scheduleRead: 'schedule:read',
  scheduleManage: 'schedule:manage',
  scheduleEventRead: 'schedule:event:read',
  scheduleEventManage: 'schedule:event:manage',
  deviceRead: 'device:read',
  deviceManage: 'device:manage',
  deviceWake: 'device:wake',
  storageRead: 'storage:read',
  storageManage: 'storage:manage',
  imageRead: 'image:read',
  imageManage: 'image:manage',
  ticketRead: 'ticket:read',
  ticketCreate: 'ticket:create',
  ticketHandle: 'ticket:handle',
  ticketManage: 'ticket:manage',
  ticketClaim: 'ticket:claim',
  ticketComment: 'ticket:comment',
  ticketTransition: 'ticket:transition',
  ticketAssign: 'ticket:assign',
  ticketUpdate: 'ticket:update',
  ticketRuleManage: 'ticket:rule:manage',
  ticketExport: 'ticket:export',
  ticketScopeManage: 'ticket:scope:manage',
  ticketScopeAll: 'ticket:scope:all',
  ticketAttachment: 'ticket:attachment',
  ticketInspectionManage: 'ticket:inspection:manage',
} as const

export type PermissionCode = (typeof PERMISSIONS)[keyof typeof PERMISSIONS]
