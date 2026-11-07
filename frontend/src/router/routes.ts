import type { RouteRecordRaw } from 'vue-router'

import DefaultLayout from '@/layouts/DefaultLayout.vue'
import { PERMISSIONS } from '@/utils/permissions'

export const routes: RouteRecordRaw[] = [
  {
    path: '/login',
    name: 'login',
    component: () => import('@/views/LoginView.vue'),
    meta: { title: '登录', public: true },
  },
  {
    path: '/',
    component: DefaultLayout,
    children: [
      {
        path: '',
        name: 'overview',
        component: () => import('@/views/OverviewView.vue'),
        meta: { title: '总览' },
      },
      {
        path: 'teacher',
        name: 'teacher',
        component: () => import('@/views/TeacherView.vue'),
        meta: { title: '教师工作台', permission: PERMISSIONS.roomRead },
      },
      {
        path: 'areas',
        name: 'areas',
        component: () => import('@/views/AreasView.vue'),
        meta: { title: '区域', permission: PERMISSIONS.areaRead },
      },
      {
        path: 'areas/:id',
        name: 'area-detail',
        component: () => import('@/views/AreaDetailView.vue'),
        meta: { title: '区域总览', permission: PERMISSIONS.areaRead, activeName: 'areas' },
      },
      {
        path: 'rooms',
        name: 'rooms',
        component: () => import('@/views/RoomsView.vue'),
        meta: { title: '房间', permission: PERMISSIONS.roomRead },
      },
      {
        path: 'rooms/:id',
        name: 'room-detail',
        component: () => import('@/views/RoomDetailView.vue'),
        meta: { title: '房间总览', permission: PERMISSIONS.roomRead, activeName: 'rooms' },
      },
      {
        path: 'devices',
        name: 'devices',
        component: () => import('@/views/DevicesView.vue'),
        meta: { title: '设备', permission: PERMISSIONS.deviceRead },
      },
      {
        path: 'devices/:serial',
        name: 'device-detail',
        component: () => import('@/views/DeviceDetailView.vue'),
        meta: { title: '设备详情', permission: PERMISSIONS.deviceRead, activeName: 'devices' },
      },
      {
        path: 'device-events',
        name: 'device-events',
        component: () => import('@/views/DeviceEventsView.vue'),
        meta: { title: '设备在线事件', permission: PERMISSIONS.deviceRead },
      },
      {
        path: 'schedules',
        name: 'schedules',
        component: () => import('@/views/ScheduleBoardView.vue'),
        meta: { title: '课表', permission: PERMISSIONS.scheduleRead },
      },
      {
        path: 'schedules/list',
        name: 'schedule-list',
        component: () => import('@/views/SchedulesView.vue'),
        meta: { title: '课表列表', permission: PERMISSIONS.scheduleRead, activeName: 'schedules' },
      },
      {
        path: 'schedules/plans',
        name: 'schedule-plans',
        component: () => import('@/views/SchedulePlansView.vue'),
        meta: {
          title: '排课计划',
          permission: PERMISSIONS.scheduleRead,
          activeName: 'schedule-plans',
        },
      },
      {
        path: 'schedules/:id',
        name: 'schedule-detail',
        component: () => import('@/views/ScheduleDetailView.vue'),
        meta: { title: '课表详情', permission: PERMISSIONS.scheduleRead, activeName: 'schedules' },
      },
      {
        path: 'events',
        name: 'events',
        component: () => import('@/views/EventsView.vue'),
        meta: { title: '触发事件', permission: PERMISSIONS.scheduleEventRead },
      },
      {
        path: 'federation',
        name: 'federation',
        component: () => import('@/views/FederationView.vue'),
        meta: { title: '总控联邦', permission: PERMISSIONS.deviceRead, adminOnly: true },
      },
      {
        path: 'machines',
        name: 'machines',
        component: () => import('@/views/MachinesView.vue'),
        meta: { title: '机器总览', permission: PERMISSIONS.deviceRead },
      },
      {
        path: 'machines/:serial',
        name: 'machine-detail',
        component: () => import('@/views/MachineDetailView.vue'),
        meta: { title: '机器详情', permission: PERMISSIONS.deviceRead, activeName: 'machines' },
      },
      {
        path: 'images',
        name: 'images',
        component: () => import('@/views/ImagesView.vue'),
        meta: { title: '镜像', permission: PERMISSIONS.imageRead },
      },
      {
        path: 'images/:id',
        name: 'image-detail',
        component: () => import('@/views/ImageDetailView.vue'),
        meta: { title: '镜像详情', permission: PERMISSIONS.imageRead, activeName: 'images' },
      },
      {
        path: 'storage',
        name: 'storage',
        component: () => import('@/views/StorageView.vue'),
        meta: { title: '存储', permission: PERMISSIONS.storageRead },
      },
      {
        path: 'agent-configs',
        name: 'agent-configs',
        component: () => import('@/views/AgentConfigsView.vue'),
        meta: { title: '节点配置', permission: PERMISSIONS.deviceRead },
      },
      {
        path: 'tickets',
        name: 'tickets',
        component: () => import('@/views/TicketsView.vue'),
        meta: { title: '工单', permission: PERMISSIONS.ticketRead },
      },
      {
        path: 'tickets/insights',
        name: 'ticket-insights',
        component: () => import('@/views/TicketInsightsView.vue'),
        meta: { title: '工单分析', permission: PERMISSIONS.ticketRead, activeName: 'tickets' },
      },
      {
        path: 'tickets/rules',
        name: 'ticket-rules',
        component: () => import('@/views/TicketRulesView.vue'),
        meta: {
          title: '自动规则',
          permissionAny: [PERMISSIONS.ticketRuleManage, PERMISSIONS.ticketManage],
          activeName: 'tickets',
        },
      },
      {
        path: 'tickets/inspections',
        name: 'ticket-inspections',
        component: () => import('@/views/TicketInspectionsView.vue'),
        meta: {
          title: '巡检计划',
          permissionAny: [PERMISSIONS.ticketInspectionManage, PERMISSIONS.ticketManage],
          activeName: 'tickets',
        },
      },
      {
        path: 'tickets/scopes',
        name: 'ticket-scopes',
        component: () => import('@/views/TicketScopesView.vue'),
        meta: {
          title: '工单可见范围',
          permissionAny: [PERMISSIONS.ticketScopeManage, PERMISSIONS.ticketManage],
          activeName: 'tickets',
        },
      },
      {
        path: 'tickets/:id',
        name: 'ticket-detail',
        component: () => import('@/views/TicketDetailView.vue'),
        meta: { title: '工单详情', permission: PERMISSIONS.ticketRead, activeName: 'tickets' },
      },
      {
        path: 'users',
        name: 'users',
        component: () => import('@/views/UsersView.vue'),
        meta: { title: '用户', permission: PERMISSIONS.userManage },
      },
      {
        path: 'roles',
        name: 'roles',
        component: () => import('@/views/RolesView.vue'),
        meta: { title: '角色与权限', permission: PERMISSIONS.roleManage },
      },
      {
        path: 'audit',
        name: 'audit',
        component: () => import('@/views/AuditView.vue'),
        meta: { title: '审计日志', permission: PERMISSIONS.auditRead },
      },
      {
        path: 'announcements',
        name: 'announcements',
        component: () => import('@/views/AnnouncementsView.vue'),
        meta: { title: '公告', permission: PERMISSIONS.announcementRead },
      },
      {
        path: 'account',
        name: 'account',
        component: () => import('@/views/AccountView.vue'),
        meta: { title: '账户' },
      },
      {
        path: 'forbidden',
        name: 'forbidden',
        component: () => import('@/views/ForbiddenView.vue'),
        meta: { title: '无权访问' },
      },
      {
        path: ':pathMatch(.*)*',
        name: 'not-found',
        component: () => import('@/views/NotFoundView.vue'),
        meta: { title: '页面不存在' },
      },
    ],
  },
]
