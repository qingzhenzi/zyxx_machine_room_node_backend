# 教师端（阶段 1）：teacher 角色 + 教师工作台

> 阶段 1 目标：在现有管理控制台里给出「教师」这一内置角色，并提供一页教师工作台，
> 复用已有 API，不新建用户体系、不引入新的鉴权模型。

## 1. 角色与权限

迁移 **0033_teacher_role.sql** 幂等创建角色 `teacher`（显示名「教师」，`is_system = FALSE`，
可由管理员在「角色权限」页面调整）：

| 权限 | 用途 |
| --- | --- |
| `area:read` / `room:read` | 查看区域/房间 |
| `schedule:read` / `schedule:event:read` | 查看课表与触发事件 |
| `device:read` | 查看设备/机器 |
| `device:wake` | 机器网络唤醒（控制台机器详情） |
| `vm:control` | 虚拟机远程控制（控制台机器详情 → 虚拟机） |
| `image:read` / `image:manage` | 查看与管理实验镜像 |
| `storage:read` | 查看对象存储 |
| `ticket:read/create/comment/claim/transition` | 工单：查看、报修、评论、认领、流转 |

说明：

- **不需要新建用户体系**：管理员在「用户」页面创建账号 → 「角色」页面把 `teacher` 授予该账号即可。
- teacher 不含 `user:manage` / `role:manage` / `audit:read` / `device:manage` 等管理权限。
- admin 仍由 bootstrap 自动全量授权（含以上所有权限）。

## 2. 教师工作台（/teacher）

路由：`/teacher`，标题「教师工作台」，路由权限 `room:read`；侧栏「工作台」分组下新增入口。

页面区块：

1. **今日课程**：调用现有排课工作台聚合接口
   `GET /api/v1/admin/schedule-board?from=<today>&to=<today>`，前端用
   `expandBoardBlocks` + `blocksOfDate` 展开今天的课块，按开始时间排序；无课/接口失败都显示友好空态。
2. **我的房间**：`GET /api/v1/admin/rooms` 最近 12 个房间，点击进入房间总览
   （房间内可继续看设备/课表/镜像）。
3. **快捷操作卡**：镜像管理、工单、设备唤醒、VM 控制（按当前账号权限动态显示；
   「设备唤醒 / VM 控制」跳到机器总览后进入机器详情操作）。
4. **实验环境模板**：占位卡片（明确标注「即将上线」），阶段 2 再接入一键创建实验环境。
5. **下发环境配置（阶段 1.5）**：选择房间 + 已发布 `vm_disk` 底包 + 规格，按房间主设备与子机
   逐槽位下发 `create`（每台 VM 命名 `<前缀>-<槽位尾号>`），展示每个 request_id 的轮询状态；
   详见 [`remote-vm-control.md`](remote-vm-control.md) §2.2。

## 3. 范围与后续

阶段 1 只做「角色 + 工作台入口 + 复用现有能力」：

- 不做教师专属的资源分配/班级成员体系（当前排课模型已有 teachers/classes/courses，可复用）；
- 「实验环境模板」目前是静态占位；
- 后续可做：按教师过滤课表/房间、模板一键部署（镜像 + VM/容器 + 网络）、课堂临时环境回收。
