/* ==========================================================================
   Room Node Console · 工单 / 工单分析 静态原型
   --------------------------------------------------------------------------
   - 纯前端：不请求任何接口、不加载外部资源，直接以 file:// 打开即可
   - hash 路由：#list（默认）/ #detail/<id> / #insights
   - 所有数据为内存 mock；新建工单、状态流转、评论只改内存，刷新即还原
   - 列表 / 详情 / 分析页共用同一份工单数据，筛选、KPI、图表口径一致
   ========================================================================== */
(function () {
  'use strict';

  /* ------------------------------ 基础工具 ------------------------------ */

  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));
  const pad2 = (n) => String(n).padStart(2, '0');
  const pad4 = (n) => String(n).padStart(4, '0');
  const clamp = (n, lo, hi) => Math.min(hi, Math.max(lo, n));

  function esc(value) {
    return String(value == null ? '' : value)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  const MIN = 60 * 1000;
  const HOUR = 60 * MIN;
  const DAY = 24 * HOUR;
  const TZ_OFFSET = 8 * HOUR; // 数据按 Asia/Shanghai（UTC+8）解释，展示与浏览器时区无关

  // '2026-10-31 09:12' → epoch（固定 UTC+8）
  function parseTs(text) {
    return Date.parse(String(text).replace(' ', 'T') + ':00+08:00');
  }

  function zoned(ts) {
    return new Date(ts + TZ_OFFSET);
  }

  function fmtDate(ts) {
    const d = zoned(ts);
    return `${d.getUTCFullYear()}-${pad2(d.getUTCMonth() + 1)}-${pad2(d.getUTCDate())}`;
  }

  function fmtDateCompact(ts) {
    return fmtDate(ts).replace(/-/g, '');
  }

  function fmtTime(ts) {
    const d = zoned(ts);
    return `${pad2(d.getUTCHours())}:${pad2(d.getUTCMinutes())}`;
  }

  function fmtDT(ts) {
    return `${fmtDate(ts)} ${fmtTime(ts)}`;
  }

  function fmtMD(ts) {
    const d = zoned(ts);
    return `${pad2(d.getUTCMonth() + 1)}/${pad2(d.getUTCDate())}`;
  }

  function fmtMDHM(ts) {
    return `${fmtMD(ts)} ${fmtTime(ts)}`;
  }

  function dayStart(ts) {
    return Math.floor((ts + TZ_OFFSET) / DAY) * DAY - TZ_OFFSET;
  }

  function dayKey(ts) {
    return fmtDate(ts);
  }

  // 相对「今天 18:35」的列表更新时间
  function fmtUpdated(ts) {
    if (fmtDate(ts) === fmtDate(NOW)) return `今天 ${fmtTime(ts)}`;
    if (fmtDate(ts) === fmtDate(NOW - DAY)) return `昨天 ${fmtTime(ts)}`;
    return fmtMDHM(ts);
  }

  function fmtDuration(ms) {
    if (ms < 0) ms = 0;
    if (ms < HOUR) return `${Math.max(1, Math.round(ms / MIN))} 分钟`;
    if (ms < DAY) return `${(ms / HOUR).toFixed(1)} 小时`;
    return `${(ms / DAY).toFixed(1)} 天`;
  }

  function fmtDurationParts(ms) {
    if (ms < HOUR) return { value: String(Math.max(1, Math.round(ms / MIN))), unit: '分钟' };
    if (ms < DAY) return { value: (ms / HOUR).toFixed(1), unit: '小时' };
    return { value: (ms / DAY).toFixed(1), unit: '天' };
  }

  /* ------------------------------ 固定时间轴 ---------------------------- */

  // 原型「当前时间」固定为 2026-11-01 18:35（周日），保证截图与口径可复现
  const NOW = parseTs('2026-11-01 18:35');
  const ME = '张伟';
  const PAGE_SIZE = 10;
  let actionTick = 0;

  // 交互动作使用「今天稍早」的时间戳，保证仍落在 NOW 之前，分析页口径一致
  function actionNow() {
    actionTick += 1;
    return Math.min(NOW, NOW - 30 * MIN + actionTick * 2 * MIN);
  }

  /* ------------------------------ 枚举字典 ------------------------------ */

  const STATUS = {
    open: { label: '待处理', tone: 'tone-amber', order: 0 },
    in_progress: { label: '处理中', tone: 'tone-accent', order: 1 },
    resolved: { label: '已解决', tone: 'tone-ok', order: 2 },
    closed: { label: '已关闭', tone: 'tone-slate', order: 3 },
    cancelled: { label: '已取消', tone: 'tone-faint', order: 4 },
  };
  const STATUS_ORDER = ['all', 'open', 'in_progress', 'resolved', 'closed', 'cancelled'];
  const STATUS_TAB_LABEL = {
    all: '全部',
    open: '待处理',
    in_progress: '处理中',
    resolved: '已解决',
    closed: '已关闭',
    cancelled: '已取消',
  };

  const PRIORITY = {
    urgent: { label: '紧急', dot: 'is-urgent', tone: 'tone-danger', sla: 4 },
    high: { label: '高', dot: 'is-high', tone: 'tone-amber', sla: 8 },
    normal: { label: '普通', dot: 'is-normal', tone: 'tone-accent', sla: 24 },
    low: { label: '低', dot: 'is-low', tone: 'tone-faint', sla: 72 },
  };
  const PRIORITY_ORDER = ['urgent', 'high', 'normal', 'low'];

  const CATEGORY = {
    machine_health: { label: '机器健康', color: '#3a5bd9' },
    device_offline: { label: '设备离线', color: '#dc6803' },
    delivery_failed: { label: '推送失败', color: '#7b6bc4' },
    distribution_failed: { label: '分发失败', color: '#b8618a' },
    security: { label: '安全', color: '#d92d20' },
    other: { label: '其他', color: '#64748b' },
  };
  const CATEGORY_ORDER = [
    'machine_health',
    'device_offline',
    'delivery_failed',
    'distribution_failed',
    'security',
    'other',
  ];

  const SOURCE = {
    auto: { label: '自动', tone: 'tone-accent', icon: 'i-robot' },
    manual: { label: '人工', tone: 'tone-slate', icon: 'i-user' },
  };

  const HANDLERS = ['张伟', '李娜', '王强'];
  const REPORTERS = ['陈静', '刘洋', '赵敏', '周慧', '吴磊', '孙浩'];

  /* ------------------------------ 房间 / 机器 --------------------------- */

  const ROOMS = [
    {
      code: 'room-101',
      name: '智慧教室 A',
      area: '教学楼 A · 3F',
      device: '中控主机',
      serial: '4f40ccef2a7b3d21',
    },
    {
      code: 'room-102',
      name: '计算机房 B',
      area: '教学楼 A · 3F',
      device: '学生终端',
      serial: '7a91d3e04c6b8f52',
    },
    {
      code: 'room-103',
      name: '多媒体教室',
      area: '教学楼 A · 3F',
      device: '投影仪',
      serial: 'b2e6a8c15d4f9073',
    },
    {
      code: 'room-104',
      name: '语音教室',
      area: '教学楼 A · 4F',
      device: '语音终端',
      serial: 'c85f1b9e7a3d2640',
    },
    {
      code: 'room-105',
      name: '研讨教室',
      area: '教学楼 B · 5F',
      device: '电子班牌',
      serial: 'd19a4c7fb62e8305',
    },
    {
      code: 'room-106',
      name: '创新工坊',
      area: '教学楼 B · 2F',
      device: '分发节点',
      serial: 'e63b8d2a51f7c094',
    },
    {
      code: 'room-107',
      name: '阶梯教室',
      area: '教学楼 B · 2F',
      device: '中控主机',
      serial: 'fa27c4e9b03d1684',
    },
    {
      code: 'room-108',
      name: '实验工作站房',
      area: '教学楼 B · 1F',
      device: '实验工作站',
      serial: '0d3e7b5ca9182f46',
    },
  ];

  const ROOM_MAP = new Map(ROOMS.map((room) => [room.code, room]));
  const roomOf = (code) => ROOM_MAP.get(code) || ROOMS[0];

  /* ------------------------------ 信号文案 ------------------------------ */

  function signalFor(seed, createdTs, priority) {
    if (seed.source !== 'auto') {
      return {
        kind: 'manual',
        label: '人工上报',
        ref: '—',
        at: createdTs,
      };
    }
    const stamp = `${fmtDateCompact(createdTs)}_${fmtTime(createdTs).replace(':', '')}`;
    switch (seed.category) {
      case 'machine_health':
        return {
          kind: 'snapshot',
          label: `机器健康检查 ${priority === 'urgent' ? 'critical' : 'warning'}`,
          ref: `snap_${stamp}_${roomOf(seed.room).serial.slice(0, 8)}`,
          at: createdTs - 2 * MIN,
        };
      case 'device_offline':
        return {
          kind: 'presence',
          label: '设备离线超过 5 分钟',
          ref: `evt_presence_${stamp}`,
          at: createdTs - 5 * MIN,
        };
      case 'delivery_failed':
        return {
          kind: 'delivery',
          label: '排课事件推送失败（重试 3 次后仍失败）',
          ref: `dlv_${stamp}_${roomOf(seed.room).serial.slice(0, 6)}`,
          at: createdTs - MIN,
        };
      case 'distribution_failed':
        return {
          kind: 'distribution',
          label: '镜像分发失败（写入 / 校验超时）',
          ref: `dist_${stamp}_${roomOf(seed.room).serial.slice(0, 6)}`,
          at: createdTs - 3 * MIN,
        };
      case 'security':
        return {
          kind: 'audit',
          label: '安全审计告警',
          ref: `audit_${stamp}`,
          at: createdTs - MIN,
        };
      default:
        return { kind: 'other', label: '自动告警', ref: `sig_${stamp}`, at: createdTs - MIN };
    }
  }

  /* ------------------------------ Mock 工单 ----------------------------- */
  // 32 张工单，创建时间均落在 2026-10-03 ~ 2026-11-01（近 30 天）。
  // comments: [评论人, 创建后多少分钟, 内容]
  const SEED = [
    {
      created: '2026-10-03 09:20',
      status: 'closed',
      priority: 'low',
      category: 'other',
      source: 'manual',
      title: '教室投影画面偶发闪烁',
      desc:
        '陈静老师反馈 room-103 投影画面在切换课件时偶发闪烁，HDMI 直连正常，怀疑 VGA 线缆或接口接触不良，已影响两节课的演示。',
      reporter: '陈静',
      assignee: '王强',
      room: 'room-103',
      device: '投影仪',
      resolved: '2026-10-04 10:05',
      resolution: '更换 VGA 线缆并重新压接接口，连续测试 2 节课画面稳定。',
    },
    {
      created: '2026-10-05 14:42',
      status: 'closed',
      priority: 'high',
      category: 'machine_health',
      source: 'auto',
      title: 'room-102 磁盘使用率 96%，触发严重告警',
      desc:
        '机器健康检查发现 room-102-node 系统盘使用率 96%，/var/log 与临时目录增长明显，继续写入可能导致遥测上报失败。',
      reporter: '系统',
      assignee: '张伟',
      room: 'room-102',
      device: '学生终端',
      resolved: '2026-10-05 16:20',
      resolution: '清理临时目录与历史日志并配置 logrotate，使用率降至 61%。',
      comments: [
        ['张伟', 25, '已清理临时文件，使用率降到 78%，继续观察。'],
        ['李娜', 60, '建议在节点配置里开启磁盘水位告警，阈值 85% 更合适。'],
      ],
    },
    {
      created: '2026-10-07 08:15',
      status: 'closed',
      priority: 'normal',
      category: 'delivery_failed',
      source: 'auto',
      title: '排课事件推送失败：软件工程 2302 周三第 3-4 节',
      desc:
        '排课事件推送至 room-105 电子班牌时返回 502，重试 3 次仍失败，教室无法显示当日课程。',
      reporter: '系统',
      assignee: '李娜',
      room: 'room-105',
      device: '电子班牌',
      resolved: '2026-10-07 12:40',
      resolution: '电子班牌服务重启后重新投递成功，已补发当日课表。',
      comments: [['李娜', 15, '已重启电子班牌服务并重新投递。']],
    },
    {
      created: '2026-10-09 10:02',
      status: 'closed',
      priority: 'urgent',
      category: 'device_offline',
      source: 'auto',
      title: 'room-104 语音终端离线超过 5 分钟',
      desc:
        '在线事件监测到 room-104 语音终端 10:02 起离线，心跳中断，可能影响听力训练与设备巡检。',
      reporter: '系统',
      assignee: '王强',
      room: 'room-104',
      device: '语音终端',
      resolved: '2026-10-09 11:05',
      resolution: '更换 PoE 交换机端口后恢复在线，离线原因继续观察。',
    },
    {
      created: '2026-10-11 15:30',
      status: 'closed',
      priority: 'normal',
      category: 'other',
      source: 'manual',
      title: '创新工坊实验台无法开机',
      desc:
        '刘洋反馈 room-106 3 号实验台按下电源无反应，配电箱指示灯正常，其余实验台可正常使用。',
      reporter: '刘洋',
      assignee: '张伟',
      room: 'room-106',
      device: '实验工作站',
      resolved: '2026-10-12 09:10',
      resolution: '更换实验台电源模块，开机自检通过。',
    },
    {
      created: '2026-10-13 09:00',
      status: 'closed',
      priority: 'high',
      category: 'machine_health',
      source: 'auto',
      title: 'room-107 中控主机内存占用持续超过 90%',
      desc:
        '机器健康检查连续 3 次采样内存占用超过 90%，中控服务响应变慢，可能影响课前设备自检。',
      reporter: '系统',
      assignee: '李娜',
      room: 'room-107',
      device: '中控主机',
      resolved: '2026-10-13 14:30',
      resolution: '重启中控服务并调整缓存上限，内存回落至 68%。',
      comments: [['李娜', 40, '重启中控服务后内存回落至 68%，继续观察一天。']],
    },
    {
      created: '2026-10-15 07:50',
      status: 'closed',
      priority: 'normal',
      category: 'distribution_failed',
      source: 'auto',
      title: '镜像「实验环境-base」分发到 room-106 失败',
      desc:
        '镜像分发任务在写入阶段超时，room-106-node 返回校验失败，实验课无法按计划切换环境。',
      reporter: '系统',
      assignee: '王强',
      room: 'room-106',
      device: '分发节点',
      resolved: '2026-10-15 10:20',
      resolution: '清理本地镜像缓存后重新分发，sha256 校验通过。',
    },
    {
      created: '2026-10-17 16:10',
      status: 'resolved',
      priority: 'high',
      category: 'machine_health',
      source: 'auto',
      title: 'room-102 机器健康检查 warning：内存占用 92%',
      desc:
        'room-102 学生终端内存占用连续两次告警，学生登录后出现黑屏，疑似终端缓存泄漏。',
      reporter: '系统',
      assignee: '张伟',
      room: 'room-102',
      device: '学生终端',
      resolved: '2026-10-18 10:35',
      resolution: '扩容学生终端服务并重启，内存回落至 70%，观察一天无复发。',
      comments: [['张伟', 30, '已扩容并重启学生终端服务，观察无黑屏。']],
    },
    {
      created: '2026-10-19 11:00',
      status: 'cancelled',
      priority: 'low',
      category: 'security',
      source: 'manual',
      title: '管理员账号异地登录告警',
      desc:
        '审计日志记录管理员账号在异地 IP 登录，来源为 VPN 出口，需确认是否为本人操作。',
      reporter: '赵敏',
      assignee: '李娜',
      room: 'room-101',
      device: '中控主机',
      cancelBy: '赵敏',
      cancelReason: '与赵敏核实为 VPN 出口切换导致的误报，取消工单。',
      comments: [['李娜', 30, '与赵敏核实为 VPN 出口切换，属误报。']],
    },
    {
      created: '2026-10-21 13:20',
      status: 'closed',
      priority: 'normal',
      category: 'device_offline',
      source: 'auto',
      title: 'room-105 电子班牌离线超过 5 分钟',
      desc:
        '电子班牌心跳中断，教室门口无法显示课程与通知，现场屏幕停留在昨日课表。',
      reporter: '系统',
      assignee: '张伟',
      room: 'room-105',
      device: '电子班牌',
      resolved: '2026-10-21 15:00',
      resolution: '重启电子班牌并升级固件，心跳恢复、课表同步正常。',
    },
    {
      created: '2026-10-23 08:05',
      status: 'resolved',
      priority: 'urgent',
      category: 'machine_health',
      source: 'auto',
      title: 'room-101 中控主机 CPU 温度 87°C',
      desc:
        '机器健康检查 critical：room-101-node CPU 温度 87°C，风扇转速异常，存在降频与宕机风险，影响上午 4 节课。',
      reporter: '系统',
      assignee: '李娜',
      room: 'room-101',
      device: '中控主机',
      resolved: '2026-10-23 09:28',
      resolution: '清理机柜积灰、调整风扇策略，温度降至 72°C，复测稳定。',
      comments: [['李娜', 20, '清理机柜积灰并调整风扇策略，温度已降至 72°C。']],
    },
    {
      created: '2026-10-24 10:40',
      status: 'resolved',
      priority: 'normal',
      category: 'other',
      source: 'manual',
      title: '语音教室功放无声音',
      desc:
        '周慧老师反馈 room-104 功放无声，麦克风与电脑音源均无输出，重启功放无效。',
      reporter: '周慧',
      assignee: '王强',
      room: 'room-104',
      device: '功放',
      resolved: '2026-10-25 09:05',
      resolution: '更换功放保险丝并恢复默认配置，试音正常。',
    },
    {
      created: '2026-10-24 18:10',
      status: 'cancelled',
      priority: 'normal',
      category: 'delivery_failed',
      source: 'auto',
      title: '排课事件推送失败：高等数学 周一第 1-2 节',
      desc:
        '周一课表事件推送超时，room-103 电子班牌未收到更新，重试队列积压 3 条。',
      reporter: '系统',
      assignee: '李娜',
      room: 'room-103',
      device: '电子班牌',
      cancelBy: '系统',
      cancelReason: '上游服务恢复后自动补发成功，取消工单。',
      comments: [['李娜', 60, '上游恢复后自动补发成功。']],
    },
    {
      created: '2026-10-26 08:20',
      status: 'resolved',
      priority: 'high',
      category: 'distribution_failed',
      source: 'auto',
      title: '镜像「Python 3.12 实验镜像」分发到 room-106 失败',
      desc:
        '镜像分发在 92% 处卡住并超时，节点磁盘剩余空间不足，实验课环境准备受阻。',
      reporter: '系统',
      assignee: '张伟',
      room: 'room-106',
      device: '分发节点',
      resolved: '2026-10-26 12:45',
      resolution: '清理旧镜像释放 18GB，重新分发成功并校验通过。',
      comments: [['张伟', 45, '重试分发成功，sha256 校验一致。']],
    },
    {
      created: '2026-10-27 09:15',
      status: 'in_progress',
      priority: 'normal',
      category: 'machine_health',
      source: 'auto',
      title: 'room-107 磁盘 inode 使用率 91%',
      desc:
        '机器健康检查发现 inode 使用率持续上升，大量小文件写入后无法创建新文件，可能影响日志与遥测上报。',
      reporter: '系统',
      assignee: '李娜',
      room: 'room-107',
      device: '中控主机',
      comments: [['李娜', 120, '已定位为录播日志未轮转，正在清理并调整轮转策略。']],
    },
    {
      created: '2026-10-28 10:30',
      status: 'in_progress',
      priority: 'high',
      category: 'device_offline',
      source: 'auto',
      title: 'room-103 投影仪离线超过 5 分钟',
      desc:
        '投影仪离线，电子班牌显示正常；现场检查电源适配器指示灯不亮，需确认是否供电故障。',
      reporter: '系统',
      assignee: '王强',
      room: 'room-103',
      device: '投影仪',
      comments: [['王强', 90, '现场电源适配器损坏，已报修更换。']],
    },
    {
      created: '2026-10-28 14:10',
      status: 'resolved',
      priority: 'low',
      category: 'other',
      source: 'manual',
      title: '实验台串口设备无法识别',
      desc:
        '吴磊反馈 room-108 实验台串口设备偶发无法识别，重插后恢复，怀疑驱动或 USB 集线器供电不足。',
      reporter: '吴磊',
      assignee: '张伟',
      room: 'room-108',
      device: '实验工作站',
      resolved: '2026-10-28 16:40',
      resolution: '更换 USB 集线器并更新串口驱动，连续测试无异常。',
    },
    {
      created: '2026-10-29 08:40',
      status: 'in_progress',
      priority: 'urgent',
      category: 'machine_health',
      source: 'auto',
      title: 'room-102 磁盘使用率 94%，写入可能失败',
      desc:
        '机器健康检查 critical：room-102-node 磁盘使用率 94%，日志写入偶发失败，晚自习上机课可能受影响。',
      reporter: '系统',
      assignee: '张伟',
      room: 'room-102',
      device: '学生终端',
      comments: [['张伟', 35, '已清理临时目录，磁盘水位回落中。']],
    },
    {
      created: '2026-10-29 16:05',
      status: 'open',
      priority: 'normal',
      category: 'delivery_failed',
      source: 'auto',
      title: '排课事件推送失败：大学物理 周五第 5-6 节',
      desc:
        '排课事件推送返回 504，room-105 电子班牌未更新课程；自动重试队列已积压。',
      reporter: '系统',
      assignee: null,
      room: 'room-105',
      device: '电子班牌',
    },
    {
      created: '2026-10-30 09:05',
      status: 'in_progress',
      priority: 'high',
      category: 'security',
      source: 'manual',
      title: '夜间门禁异常开启：room-101',
      desc:
        '门禁记录显示 23:41 room-101 门禁被异常开启，无对应刷卡记录，需现场核查与调取监控。',
      reporter: '赵敏',
      assignee: '王强',
      room: 'room-101',
      device: '门禁',
      comments: [['王强', 50, '已更换门禁读卡器，等待夜间验证。']],
    },
    {
      created: '2026-10-30 10:20',
      status: 'open',
      priority: 'high',
      category: 'distribution_failed',
      source: 'auto',
      title: '镜像「AI 实验镜像」分发到 room-106 超时',
      desc:
        '镜像分发任务超时，room-106-node 磁盘写入速度异常，实验课镜像准备未完成。',
      reporter: '系统',
      assignee: '李娜',
      room: 'room-106',
      device: '分发节点',
    },
    {
      created: '2026-10-30 15:45',
      status: 'open',
      priority: 'low',
      category: 'other',
      source: 'manual',
      title: '教室无线麦克风无声',
      desc:
        '孙浩反馈 room-104 无线麦克风无声，更换电池后仍无输出，可能为接收器故障。',
      reporter: '孙浩',
      assignee: null,
      room: 'room-104',
      device: '无线麦克风',
    },
    {
      created: '2026-10-31 07:55',
      status: 'in_progress',
      priority: 'urgent',
      category: 'device_offline',
      source: 'auto',
      title: 'room-108 实验工作站离线超过 5 分钟',
      desc:
        '实验工作站离线，实验数据采集中断，影响下午 2 个班级的实验课。',
      reporter: '系统',
      assignee: '王强',
      room: 'room-108',
      device: '实验工作站',
      comments: [['王强', 25, '交换机端口异常，已联系网络组处理。']],
    },
    {
      created: '2026-10-31 09:10',
      status: 'open',
      priority: 'high',
      category: 'machine_health',
      source: 'auto',
      title: 'room-107 磁盘使用率 93%',
      desc:
        '机器健康检查 warning：磁盘使用率 93%，临时文件与录播缓存持续增长。',
      reporter: '系统',
      assignee: null,
      room: 'room-107',
      device: '中控主机',
    },
    {
      created: '2026-10-31 10:35',
      status: 'open',
      priority: 'normal',
      category: 'delivery_failed',
      source: 'auto',
      title: '排课事件推送失败：数据结构 周二第 3-4 节',
      desc:
        '推送失败，room-103 电子班牌课程未更新；自动重试 3 次均超时。',
      reporter: '系统',
      assignee: null,
      room: 'room-103',
      device: '电子班牌',
    },
    {
      created: '2026-10-31 11:20',
      status: 'cancelled',
      priority: 'normal',
      category: 'distribution_failed',
      source: 'auto',
      title: '镜像「办公软件套件」分发校验失败（误报）',
      desc:
        '镜像分发后 sha256 校验不一致，复查发现为节点上报延迟导致的校验时序问题。',
      reporter: '系统',
      assignee: '王强',
      room: 'room-102',
      device: '学生终端',
      cancelBy: '王强',
      cancelReason: '复核校验日志与镜像摘要一致，判定为误报，取消工单。',
      comments: [['王强', 20, '复核校验日志与镜像摘要一致，判定为误报。']],
    },
    {
      created: '2026-11-01 08:30',
      status: 'in_progress',
      priority: 'urgent',
      category: 'machine_health',
      source: 'auto',
      title: 'room-101 机器健康检查 critical：GPU 温度 89°C',
      desc:
        'GPU 温度持续超过阈值，机房空调回风温度偏高，可能触发降频，影响图形工作站课程。',
      reporter: '系统',
      assignee: '张伟',
      room: 'room-101',
      device: '中控主机',
      comments: [
        ['张伟', 12, '已限制 GPU 任务并联系物业调整空调，温度回落中。'],
        ['李娜', 40, '物业已到现场检查空调回风，预计一小时内恢复。'],
      ],
    },
    {
      created: '2026-11-01 09:05',
      status: 'open',
      priority: 'urgent',
      category: 'device_offline',
      source: 'auto',
      title: 'room-104 语音终端离线超过 5 分钟',
      desc:
        '语音终端离线，教室无法进行听力训练；同房间功放在线，初步判断终端电源异常。',
      reporter: '系统',
      assignee: null,
      room: 'room-104',
      device: '语音终端',
    },
    {
      created: '2026-11-01 10:10',
      status: 'open',
      priority: 'normal',
      category: 'security',
      source: 'manual',
      title: '未授权 USB 设备接入 room-102 学生终端',
      desc:
        '安全审计发现未登记 USB 存储设备接入学生终端，需现场核查并确认是否存在数据外带风险。',
      reporter: '赵敏',
      assignee: '李娜',
      room: 'room-102',
      device: '学生终端',
    },
    {
      created: '2026-11-01 10:45',
      status: 'resolved',
      priority: 'high',
      category: 'machine_health',
      source: 'auto',
      title: 'room-108 磁盘使用率 95%',
      desc:
        '机器健康检查 warning：磁盘使用率 95%，实验数据目录增长迅速。',
      reporter: '系统',
      assignee: '王强',
      room: 'room-108',
      device: '实验工作站',
      resolved: '2026-11-01 15:20',
      resolution: '清理过期实验数据并归档到存储节点，使用率降至 72%。',
      comments: [['王强', 120, '清理过期实验数据并归档，使用率降至 72%。']],
    },
    {
      created: '2026-11-01 13:20',
      status: 'open',
      priority: 'normal',
      category: 'delivery_failed',
      source: 'auto',
      title: '排课事件推送失败：大学英语 周四第 7-8 节',
      desc:
        '排课事件推送失败，room-106 电子班牌未更新课程，学生反馈走错教室。',
      reporter: '系统',
      assignee: '张伟',
      room: 'room-106',
      device: '电子班牌',
    },
    {
      created: '2026-10-31 22:10',
      status: 'closed',
      priority: 'high',
      category: 'distribution_failed',
      source: 'auto',
      title: '镜像「考试环境镜像」分发到 room-107 失败',
      desc:
        '考试环境镜像分发失败，节点返回空间不足；明日有考试安排，需优先处理。',
      reporter: '系统',
      assignee: '王强',
      room: 'room-107',
      device: '分发节点',
      resolved: '2026-11-01 00:40',
      resolution: '清理本地镜像缓存并重新分发，考试环境校验通过。',
      comments: [['王强', 30, '镜像重新推送成功，考试环境校验通过。']],
    },
  ];

  /* --------------------------- 工单构建 / 派生 -------------------------- */

  let ticketSeq = 0;

  function buildTimeline(seed, meta) {
    const { createdTs, resolvedTs, priority, status, assignee, room, signal } = meta;
    const entries = [];
    const auto = seed.source === 'auto';

    entries.push({
      at: createdTs,
      actor: auto ? '系统' : seed.reporter,
      kind: auto ? 'system' : 'created',
      action: auto ? '自动创建工单' : '创建工单',
      text: auto ? `触发规则：${signal.label}` : '人工上报工单',
      system: auto,
    });

    let cursor = createdTs;
    if (assignee) {
      const delay = auto ? 2 + (meta.seedIndex % 3) : 10 + (meta.seedIndex % 20);
      cursor = createdTs + delay * MIN;
      entries.push({
        at: cursor,
        actor: auto ? '系统' : assignee,
        kind: assignee && auto ? 'system' : 'assigned',
        action: auto ? `按分类自动指派给 ${assignee}` : `${assignee} 认领工单`,
        text: '',
        system: auto,
      });
    }

    const started = status === 'in_progress' || status === 'resolved' || status === 'closed';
    let startedAt = 0;
    if (started && assignee) {
      cursor = Math.max(cursor + (6 + (meta.seedIndex % 18)) * MIN, createdTs + 20 * MIN);
      startedAt = cursor;
      entries.push({
        at: cursor,
        actor: assignee,
        kind: 'started',
        action: '开始处理',
        text: '',
        system: false,
      });
    }

    if (resolvedTs) {
      entries.push({
        at: resolvedTs,
        actor: assignee || ME,
        kind: 'resolved',
        action: '标记为已解决',
        text: seed.resolution || '问题已处理。',
        system: false,
      });
    }

    if (status === 'closed' && resolvedTs) {
      const closeAt = Math.min(
        resolvedTs + (2 + (meta.seedIndex % 5)) * HOUR,
        NOW - 5 * MIN
      );
      entries.push({
        at: Math.max(closeAt, resolvedTs + 5 * MIN),
        actor: assignee || ME,
        kind: 'closed',
        action: '关闭工单',
        text: '确认问题已解决，关闭工单。',
        system: false,
      });
    }

    if (status === 'cancelled') {
      const cancelAt = Math.min(
        createdTs + (90 + (meta.seedIndex % 4) * 30) * MIN,
        NOW - 5 * MIN
      );
      entries.push({
        at: cancelAt,
        actor: seed.cancelBy || seed.reporter,
        kind: 'cancelled',
        action: '取消工单',
        text: seed.cancelReason || '经核实无需处理，取消工单。',
        system: seed.cancelBy === '系统',
      });
    }

    (seed.comments || []).forEach(([by, offset, text]) => {
      let at = createdTs + offset * MIN;
      if (startedAt) at = Math.max(at, startedAt + 2 * MIN);
      if (resolvedTs) at = Math.min(at, resolvedTs - 2 * MIN);
      entries.push({
        at,
        actor: by,
        kind: 'comment',
        action: '发表评论',
        text,
        system: false,
      });
    });

    // 未关闭且已超 SLA 的工单补一条系统升级提醒，体现「自动升级」设计
    const dueTs = createdTs + PRIORITY[priority].sla * HOUR;
    if ((status === 'open' || status === 'in_progress') && dueTs < NOW) {
      entries.push({
        at: dueTs,
        actor: '系统',
        kind: 'system',
        action: 'SLA 超时升级提醒',
        text: '工单已超过 SLA 时限，仍未关闭；系统自动升级提醒并通知运维负责人。',
        system: true,
      });
    }

    entries.sort((a, b) => a.at - b.at || (a.kind === 'comment' ? 1 : -1));
    return entries;
  }

  function normalizeSeed(seed, index) {
    const createdTs = parseTs(seed.created);
    const resolvedTs = seed.resolved ? parseTs(seed.resolved) : null;
    const room = roomOf(seed.room);
    const priority = seed.priority;
    const signal = signalFor(seed, createdTs, priority);
    const status = seed.status;
    const assignee = seed.assignee || null;

    const meta = {
      createdTs,
      resolvedTs,
      priority,
      status,
      assignee,
      room,
      signal,
      seedIndex: index,
    };

    const timeline = buildTimeline(seed, meta);
    const updatedTs = timeline.reduce((max, entry) => Math.max(max, entry.at), createdTs);

    return {
      id: '',
      code: '',
      title: seed.title,
      description: seed.desc,
      status,
      priority,
      category: seed.category,
      source: seed.source,
      assignee,
      reporter: seed.source === 'auto' ? '系统' : seed.reporter,
      room: seed.room,
      device: seed.device || room.device,
      serial: room.serial,
      hostname: `${seed.room}-node`,
      area: room.area,
      signal,
      createdTs,
      updatedTs,
      resolvedTs,
      dueTs: createdTs + PRIORITY[priority].sla * HOUR,
      timeline,
      resolution: seed.resolution || '',
      seed,
    };
  }

  const tickets = SEED.map((seed, index) => normalizeSeed(seed, index));

  // id 按创建时间升序（show 列表按 updatedTs 倒序）
  tickets.sort((a, b) => a.createdTs - b.createdTs);
  tickets.forEach((ticket, index) => {
    ticket.id = `t${pad2(index + 1)}`;
  });
  ticketSeq = tickets.length;

  // 工单编号格式：T-YYYYMMDD-####（按创建日流水）
  (function assignCodes() {
    const counters = new Map();
    tickets
      .slice()
      .sort((a, b) => a.createdTs - b.createdTs)
      .forEach((ticket) => {
        const key = fmtDateCompact(ticket.createdTs);
        const next = (counters.get(key) || 0) + 1;
        counters.set(key, next);
        ticket.code = `T-${key}-${pad4(next)}`;
      });
  })();

  const ticketById = new Map(tickets.map((ticket) => [ticket.id, ticket]));

  function findTicket(raw) {
    if (!raw) return null;
    return ticketById.get(raw) || tickets.find((ticket) => ticket.code === raw) || null;
  }

  /* ------------------------------ 视图状态 ------------------------------ */

  const state = {
    status: 'all',
    priority: 'all',
    category: 'all',
    assignee: 'all',
    search: '',
    range: 'all',
    page: 1,
    insightsDays: 30,
  };

  function matchesKeyword(ticket, keyword) {
    if (!keyword) return true;
    const room = roomOf(ticket.room);
    const haystack = [
      ticket.code,
      ticket.title,
      ticket.description,
      ticket.room,
      room.name,
      ticket.device,
      ticket.serial,
      ticket.hostname,
      ticket.assignee || '未分配',
      ticket.reporter,
      CATEGORY[ticket.category].label,
      ticket.signal.label,
      ticket.signal.ref,
    ]
      .join(' ')
      .toLowerCase();
    return haystack.includes(keyword);
  }

  function matchesNonStatus(ticket) {
    if (state.priority !== 'all' && ticket.priority !== state.priority) return false;
    if (state.category !== 'all' && ticket.category !== state.category) return false;
    if (state.assignee !== 'all') {
      if (state.assignee === 'none' && ticket.assignee) return false;
      if (state.assignee !== 'none' && ticket.assignee !== state.assignee) return false;
    }
    if (state.range !== 'all') {
      const days = Number(state.range);
      if (ticket.createdTs < NOW - days * DAY) return false;
    }
    return matchesKeyword(ticket, state.search.toLowerCase());
  }

  function filteredTickets(includeStatus = true) {
    return tickets
      .filter(
        (ticket) =>
          matchesNonStatus(ticket) &&
          (!includeStatus || state.status === 'all' || ticket.status === state.status)
      )
      .sort((a, b) => b.updatedTs - a.updatedTs);
  }

  function statusCounts() {
    const base = tickets.filter(matchesNonStatus);
    const counts = { all: base.length };
    STATUS_ORDER.slice(1).forEach((key) => {
      counts[key] = base.filter((ticket) => ticket.status === key).length;
    });
    return counts;
  }

  function isOverdue(ticket) {
    if (ticket.status === 'cancelled') return false;
    const end = ticket.resolvedTs || NOW;
    return ticket.dueTs < end;
  }

  function overdueCount() {
    return tickets.filter(isOverdue).length;
  }

  /* ------------------------------ 徽标渲染 ------------------------------ */

  function statusBadge(status) {
    const meta = STATUS[status];
    return `<span class="badge ${meta.tone}"><span class="dot"></span>${meta.label}</span>`;
  }

  function priorityCell(priority) {
    const meta = PRIORITY[priority];
    return `<span class="prio"><span class="prio-dot ${meta.dot}"></span>${meta.label}</span>`;
  }

  function categoryCell(category) {
    const meta = CATEGORY[category];
    return `<span class="cat-label"><span class="dot" style="background:${meta.color}"></span>${meta.label}</span>`;
  }

  function sourceBadge(source) {
    const meta = SOURCE[source];
    return `<span class="badge badge-source ${meta.tone}"><svg class="ico"><use href="#${meta.icon}"/></svg>${meta.label}</span>`;
  }

  function personCell(name, opts = {}) {
    if (!name) {
      return '<span class="person"><span class="avatar-xs">—</span><span class="person-name is-empty">未分配</span></span>';
    }
    const me = name === ME ? ' is-me' : '';
    return `<span class="person"><span class="avatar-xs${me}">${esc(name.slice(0, 1))}</span><span class="person-name">${esc(name)}</span></span>`;
  }

  /* ------------------------------ Toast -------------------------------- */

  function toast(message, kind = 'ok') {
    const wrap = $('#toast-wrap');
    if (!wrap) return;
    const el = document.createElement('div');
    el.className = `toast${kind === 'warn' ? ' is-warn' : ''}`;
    el.innerHTML = `<svg class="ico"><use href="#${kind === 'warn' ? 'i-alert' : 'i-check-circle'}"/></svg><span>${esc(message)}</span>`;
    wrap.appendChild(el);
    while (wrap.children.length > 3) wrap.firstElementChild.remove();
    setTimeout(() => {
      el.classList.add('is-out');
      setTimeout(() => el.remove(), 220);
    }, 2600);
  }

  /* ------------------------------ 公共片段 ------------------------------ */

  function pageHead(title, subtitle, actionsHtml) {
    return `
      <div class="page-head">
        <div class="page-head-main">
          <h1>${esc(title)}</h1>
          <p class="page-sub">${esc(subtitle)}</p>
        </div>
        <div class="page-head-actions">${actionsHtml || ''}</div>
      </div>`;
  }

  function selectOptions(items, current) {
    return items
      .map(
        (item) =>
          `<option value="${esc(item.value)}"${item.value === current ? ' selected' : ''}>${esc(item.label)}</option>`
      )
      .join('');
  }

  /* ------------------------------ 列表页 ------------------------------- */

  const KPI_ICONS = {
    open: 'i-inbox',
    in_progress: 'i-clock',
    resolved: 'i-check-circle',
    overdue: 'i-alert',
  };

  function renderList() {
    const counts = tickets.reduce(
      (acc, ticket) => {
        acc[ticket.status] += 1;
        return acc;
      },
      { open: 0, in_progress: 0, resolved: 0, closed: 0, cancelled: 0 }
    );
    const overdue = overdueCount();
    const resolveRate = Math.round((counts.resolved / tickets.length) * 100);
    const unassigned = tickets.filter((ticket) => ticket.status === 'open' && !ticket.assignee).length;
    const overdueActive = tickets.filter(
      (ticket) => isOverdue(ticket) && (ticket.status === 'open' || ticket.status === 'in_progress')
    ).length;
    const urgentInProgress = tickets.filter(
      (ticket) => ticket.status === 'in_progress' && ticket.priority === 'urgent'
    ).length;

    const kpis = [
      {
        tone: 'amber',
        icon: KPI_ICONS.open,
        label: '待处理',
        value: counts.open,
        sub: `其中 <b>${unassigned}</b> 张未分配处理人`,
      },
      {
        tone: 'accent',
        icon: KPI_ICONS.in_progress,
        label: '处理中',
        value: counts.in_progress,
        sub: `其中紧急 <b>${urgentInProgress}</b> 张，建议优先处理`,
      },
      {
        tone: 'ok',
        icon: KPI_ICONS.resolved,
        label: '已解决',
        value: counts.resolved,
        sub: `解决率 <b>${resolveRate}%</b> · 待关闭 ${counts.resolved} 张`,
      },
      {
        tone: 'danger',
        icon: KPI_ICONS.overdue,
        label: '超时',
        value: overdue,
        sub: `其中仍在处理 <b>${overdueActive}</b> 张 · 占全部 ${Math.round((overdue / tickets.length) * 100)}%`,
      },
    ];

    $('#page').innerHTML = `
      ${pageHead(
        '工单',
        '统一承接机器健康、设备离线、推送 / 分发失败与人工上报，形成可追踪的处理闭环。',
        `<button class="btn btn-outline btn-icon" type="button" title="刷新数据" data-action="refresh">
           <svg class="ico"><use href="#i-refresh"/></svg>
         </button>
         <button class="btn btn-primary" type="button" data-action="new">
           <svg class="ico"><use href="#i-plus"/></svg><span>新建工单</span>
         </button>`
      )}
      <div class="page-body">
        <div class="page-body-inner">
          <div class="kpi-strip">
            ${kpis
              .map(
                (kpi) => `
              <div class="kpi is-${kpi.tone}">
                <div class="kpi-main">
                  <div class="kpi-label">${kpi.label}</div>
                  <div class="kpi-value">${kpi.value}</div>
                  <div class="kpi-sub">${kpi.sub}</div>
                </div>
                <span class="kpi-ico"><svg class="ico"><use href="#${kpi.icon}"/></svg></span>
              </div>`
              )
              .join('')}
          </div>

          <section class="card list-card">
            <div id="list-tabs"></div>
            <div class="filters">
              <label class="filter-field">
                <span>优先级</span>
                <select data-filter="priority">
                  ${selectOptions(
                    [{ value: 'all', label: '全部' }, ...PRIORITY_ORDER.map((key) => ({ value: key, label: PRIORITY[key].label }))],
                    state.priority
                  )}
                </select>
              </label>
              <label class="filter-field">
                <span>分类</span>
                <select data-filter="category">
                  ${selectOptions(
                    [{ value: 'all', label: '全部' }, ...CATEGORY_ORDER.map((key) => ({ value: key, label: CATEGORY[key].label }))],
                    state.category
                  )}
                </select>
              </label>
              <label class="filter-field">
                <span>处理人</span>
                <select data-filter="assignee">
                  ${selectOptions(
                    [
                      { value: 'all', label: '全部' },
                      ...HANDLERS.map((name) => ({ value: name, label: name })),
                      { value: 'none', label: '未分配' },
                    ],
                    state.assignee
                  )}
                </select>
              </label>
              <label class="filter-search">
                <svg class="ico"><use href="#i-search"/></svg>
                <input
                  type="search"
                  id="list-search"
                  value="${esc(state.search)}"
                  placeholder="搜索编号 / 标题 / 房间 / 设备"
                  autocomplete="off"
                />
              </label>
              <div class="filter-spacer"></div>
              <label class="filter-field">
                <span>时间</span>
                <select data-filter="range">
                  ${selectOptions(
                    [
                      { value: 'all', label: '全部时间' },
                      { value: '7', label: '近 7 天' },
                      { value: '30', label: '近 30 天' },
                    ],
                    state.range
                  )}
                </select>
              </label>
              <button class="btn btn-ghost btn-xs" type="button" data-action="reset" id="btn-reset-filter">
                重置
              </button>
            </div>
            <div id="list-body"></div>
          </section>
        </div>
      </div>`;

    refreshList();
  }

  function tabsHtml() {
    const counts = statusCounts();
    return `
      <div class="list-toolbar">
        <div class="list-tabs" role="tablist" aria-label="工单状态">
          ${STATUS_ORDER.map((key) => {
            const active = state.status === key ? ' is-active' : '';
            const label = key === 'all' ? '全部' : STATUS[key].label;
            return `<button class="tab${active}" type="button" role="tab" data-action="tab" data-status="${key}">
              ${label}<span class="tab-count">${counts[key]}</span>
            </button>`;
          }).join('')}
        </div>
        <div class="filter-spacer"></div>
        <span class="filter-count">按更新时间倒序</span>
      </div>`;
  }

  function pagerHtml(total, pages) {
    const buttons = [];
    const windowSize = 5;
    let start = Math.max(1, state.page - 2);
    let end = Math.min(pages, start + windowSize - 1);
    start = Math.max(1, end - windowSize + 1);
    for (let page = start; page <= end; page += 1) {
      buttons.push(
        `<button class="page-btn${page === state.page ? ' is-active' : ''}" type="button" data-action="page" data-page="${page}">${page}</button>`
      );
    }
    return `
      <div class="pager">
        <div class="pager-info">共 <b>${total}</b> 张工单 · 第 <b>${state.page}</b> / <b>${pages}</b> 页</div>
        <div class="pager-btns">
          <button class="page-btn" type="button" data-action="page" data-page="${state.page - 1}" ${state.page <= 1 ? 'disabled' : ''} title="上一页">
            <svg class="ico"><use href="#i-chev-left"/></svg>
          </button>
          ${buttons.join('')}
          <button class="page-btn" type="button" data-action="page" data-page="${state.page + 1}" ${state.page >= pages ? 'disabled' : ''} title="下一页">
            <svg class="ico"><use href="#i-chev-right"/></svg>
          </button>
        </div>
      </div>`;
  }

  function emptyHtml() {
    const hasFilters =
      state.status !== 'all' ||
      state.priority !== 'all' ||
      state.category !== 'all' ||
      state.assignee !== 'all' ||
      state.search ||
      state.range !== 'all';
    return `
      <div class="empty">
        <span class="empty-ico"><svg class="ico"><use href="#i-inbox"/></svg></span>
        <div class="empty-title">没有匹配的工单</div>
        <div class="empty-sub">${
          hasFilters
            ? '当前筛选条件下没有结果，试试调整筛选或清空搜索关键词。'
            : '还没有任何工单，可手动新建一张。'
        }</div>
        <div class="empty-actions">
          ${hasFilters ? '<button class="btn btn-outline btn-sm" type="button" data-action="reset">清除筛选</button>' : ''}
          <button class="btn btn-primary btn-sm" type="button" data-action="new">
            <svg class="ico"><use href="#i-plus"/></svg>新建工单
          </button>
        </div>
      </div>`;
  }

  function rowsHtml(rows) {
    return rows
      .map(
        (ticket) => `
        <tr data-action="goto-detail" data-id="${ticket.id}">
          <td><span class="cell-code">${esc(ticket.code)}</span></td>
          <td><span class="cell-title" title="${esc(ticket.title)}">${esc(ticket.title)}</span></td>
          <td>${statusBadge(ticket.status)}</td>
          <td>${priorityCell(ticket.priority)}</td>
          <td>${categoryCell(ticket.category)}</td>
          <td>
            <div class="cell-object-room">${esc(ticket.room)}</div>
            <div class="cell-object-sub">${esc(ticket.device)}</div>
          </td>
          <td>${sourceBadge(ticket.source)}</td>
          <td>${personCell(ticket.assignee)}</td>
          <td><span class="cell-time${fmtDate(ticket.updatedTs) === fmtDate(NOW) ? ' is-today' : ''}">${fmtUpdated(ticket.updatedTs)}</span></td>
        </tr>`
      )
      .join('');
  }

  function refreshList() {
    const tabs = $('#list-tabs');
    const body = $('#list-body');
    if (!tabs || !body) return;
    const rows = filteredTickets();
    const pages = Math.max(1, Math.ceil(rows.length / PAGE_SIZE));
    state.page = clamp(state.page, 1, pages);
    const pageRows = rows.slice((state.page - 1) * PAGE_SIZE, state.page * PAGE_SIZE);
    tabs.innerHTML = tabsHtml();
    body.innerHTML = rows.length
      ? `<div class="table-wrap">
          <table class="data-table">
            <colgroup>
              <col style="width:112px" />
              <col />
              <col style="width:86px" />
              <col style="width:72px" />
              <col style="width:92px" />
              <col style="width:148px" />
              <col style="width:74px" />
              <col style="width:92px" />
              <col style="width:100px" />
            </colgroup>
            <thead>
              <tr>
                <th>编号</th>
                <th>标题</th>
                <th>状态</th>
                <th>优先级</th>
                <th>分类</th>
                <th>对象（房间 · 设备）</th>
                <th>来源</th>
                <th>处理人</th>
                <th>更新时间</th>
              </tr>
            </thead>
            <tbody>${rowsHtml(pageRows)}</tbody>
          </table>
        </div>
        ${pagerHtml(rows.length, pages)}`
      : emptyHtml();

    const reset = $('#btn-reset-filter');
    if (reset) {
      reset.hidden = !(
        state.priority !== 'all' ||
        state.category !== 'all' ||
        state.assignee !== 'all' ||
        state.search ||
        state.range !== 'all'
      );
    }
  }

  function resetFilters() {
    state.status = 'all';
    state.priority = 'all';
    state.category = 'all';
    state.assignee = 'all';
    state.search = '';
    state.range = 'all';
    state.page = 1;
  }

  /* ------------------------------ 详情页 ------------------------------- */

  function actionButtons(ticket) {
    const actions = [];
    if (ticket.status === 'open') {
      actions.push({ action: 'start', label: '开始处理', cls: 'btn-primary', icon: 'i-play' });
      actions.push({ action: 'claim', label: '认领', cls: 'btn-outline', icon: 'i-user' });
      actions.push({ action: 'cancel', label: '取消', cls: 'btn-danger', icon: 'i-x' });
    } else if (ticket.status === 'in_progress') {
      actions.push({ action: 'resolve', label: '标记解决', cls: 'btn-primary', icon: 'i-check' });
      actions.push({ action: 'cancel', label: '取消', cls: 'btn-danger', icon: 'i-x' });
    } else if (ticket.status === 'resolved') {
      actions.push({ action: 'close', label: '关闭', cls: 'btn-primary', icon: 'i-check-circle' });
      actions.push({ action: 'reopen', label: '重开', cls: 'btn-outline', icon: 'i-refresh' });
    } else if (ticket.status === 'closed' || ticket.status === 'cancelled') {
      actions.push({ action: 'reopen', label: '重开', cls: 'btn-outline', icon: 'i-refresh' });
    }
    return actions
      .map(
        (item) =>
          `<button class="btn ${item.cls}" type="button" data-action="${item.action}" data-id="${ticket.id}">
            <svg class="ico"><use href="#${item.icon}"/></svg><span>${item.label}</span>
          </button>`
      )
      .join('');
  }

  function healthOf(ticket) {
    if (ticket.category === 'machine_health') {
      return ticket.priority === 'urgent'
        ? { label: '严重', tone: 'tone-danger' }
        : { label: '警告', tone: 'tone-amber' };
    }
    if (ticket.category === 'device_offline') return { label: '离线', tone: 'tone-danger' };
    if (ticket.category === 'delivery_failed' || ticket.category === 'distribution_failed') {
      return { label: '告警', tone: 'tone-amber' };
    }
    return { label: '健康', tone: 'tone-ok' };
  }

  function onlineOf(ticket) {
    if (ticket.category === 'device_offline') return { label: '离线', tone: 'tone-danger' };
    return { label: '在线', tone: 'tone-ok' };
  }

  function timelineHtml(ticket) {
    const entries = ticket.timeline;
    if (!entries.length) return '<div class="tl-empty">暂无处理记录</div>';
    return entries
      .map((entry) => {
        const isSystem = entry.system || entry.actor === '系统';
        let dotClass = 'is-status';
        let dotContent = '<svg class="ico"><use href="#i-play"/></svg>';
        if (entry.kind === 'created') {
          dotClass = 'is-created';
          dotContent = '<svg class="ico"><use href="#i-plus"/></svg>';
        } else if (entry.kind === 'comment') {
          dotClass = 'is-comment';
          dotContent = esc(entry.actor.slice(0, 1));
        } else if (entry.kind === 'resolved' || entry.kind === 'closed') {
          dotClass = 'is-resolved';
          dotContent = '<svg class="ico"><use href="#i-check"/></svg>';
        } else if (entry.kind === 'cancelled') {
          dotClass = 'is-status';
          dotContent = '<svg class="ico"><use href="#i-x"/></svg>';
        } else if (isSystem) {
          dotClass = 'is-system';
          dotContent = '<svg class="ico"><use href="#i-robot"/></svg>';
        } else if (entry.kind === 'assigned') {
          dotContent = '<svg class="ico"><use href="#i-user"/></svg>';
        }
        return `
          <div class="tl-item${isSystem ? ' is-system' : ''}">
            <span class="tl-dot ${dotClass}">${dotContent}</span>
            <div class="tl-main">
              <div class="tl-head">
                <span class="tl-actor${isSystem ? ' is-system' : ''}">${esc(entry.actor)}</span>
                <span class="tl-action">${esc(entry.action)}</span>
                ${isSystem ? '<span class="tl-tag">系统</span>' : ''}
                <span class="tl-time">${fmtMDHM(entry.at)}</span>
              </div>
              ${entry.kind === 'comment' ? `<div class="tl-comment">${esc(entry.text)}</div>` : entry.text ? `<div class="tl-text">${esc(entry.text)}</div>` : ''}
            </div>
          </div>`;
      })
      .join('');
  }

  function slaHtml(ticket) {
    const due = ticket.dueTs;
    const created = ticket.createdTs;
    const meta = PRIORITY[ticket.priority];
    let stateText = '';
    let stateCls = 'is-safe';
    let fillCls = '';
    let ratio = 0;
    let footText = '';

    if (ticket.resolvedTs) {
      const spent = ticket.resolvedTs - created;
      ratio = clamp(spent / (due - created), 0, 1);
      if (ticket.resolvedTs > due) {
        stateText = '已超时解决';
        stateCls = 'is-over';
        fillCls = 'is-over';
      } else {
        stateText = '按时解决';
        stateCls = 'is-safe';
      }
      footText = `处理耗时 ${fmtDuration(spent)}`;
    } else if (ticket.status === 'cancelled') {
      stateText = '已取消';
      stateCls = 'is-warn';
      ratio = clamp((ticket.updatedTs - created) / (due - created), 0, 1);
      footText = '取消后不再计入 SLA';
    } else {
      ratio = clamp((NOW - created) / (due - created), 0, 1);
      if (NOW > due) {
        stateText = `已超时 ${fmtDuration(NOW - due)}`;
        stateCls = 'is-over';
        fillCls = 'is-over';
      } else if (due - NOW < 4 * HOUR) {
        stateText = `剩余 ${fmtDuration(due - NOW)}`;
        stateCls = 'is-warn';
        fillCls = 'is-warn';
      } else {
        stateText = `剩余 ${fmtDuration(due - NOW)}`;
      }
      footText = `已消耗 ${Math.round(ratio * 100)}% 时限`;
    }

    return `
      <div class="sla-top">
        <span class="sla-due mono">${fmtDT(due)}</span>
        <span class="sla-state ${stateCls}">${stateText}</span>
      </div>
      <div class="sla-track"><div class="sla-fill ${fillCls}" style="width:${(ratio * 100).toFixed(1)}%"></div></div>
      <div class="sla-foot">
        <span>${footText}</span>
        <span>${meta.label} · SLA ${meta.sla}h</span>
      </div>`;
  }

  function renderDetail(rawId, keepScroll = false) {
    const ticket = findTicket(rawId);
    if (!ticket) {
      location.hash = '#list';
      return;
    }
    const room = roomOf(ticket.room);
    const health = healthOf(ticket);
    const online = onlineOf(ticket);
    const commentCount = ticket.timeline.filter((entry) => entry.kind === 'comment').length;
    const canComment = ticket.status !== 'closed' && ticket.status !== 'cancelled';
    const statusMeta = STATUS[ticket.status];
    const activeOverdue = isOverdue(ticket);
    const oldScroll = keepScroll ? $('.page-body') && $('.page-body').scrollTop : 0;

    $('#page').innerHTML = `
      <div class="page-body">
        <div class="page-body-inner">
          <div class="detail-top">
            <a class="back-link" href="#list"><svg class="ico"><use href="#i-arrow-left"/></svg>返回工单列表</a>
            <span class="detail-hint">最近更新 ${fmtDT(ticket.updatedTs)}${activeOverdue ? ' · 已超 SLA' : ''} · 当前处理人 ${ticket.assignee ? esc(ticket.assignee) : '未分配'}</span>
          </div>

          <section class="card detail-head">
            <div class="dh-main">
              <div class="dh-top">
                <span class="dh-code">${esc(ticket.code)}</span>
                ${statusBadge(ticket.status)}
                <span class="badge ${PRIORITY[ticket.priority].tone}">优先级 · ${PRIORITY[ticket.priority].label}</span>
                <span class="badge tone-slate">${CATEGORY[ticket.category].label}</span>
                ${ticket.source === 'auto' ? '<span class="badge tone-accent"><svg class="ico"><use href="#i-robot"/></svg>自动创建</span>' : '<span class="badge tone-slate"><svg class="ico"><use href="#i-user"/></svg>人工上报</span>'}
              </div>
              <h1 class="dh-title">${esc(ticket.title)}</h1>
              <div class="dh-meta">
                <span>上报人 <b>${esc(ticket.reporter)}</b></span>
                <span class="sep">·</span>
                <span>创建于 <b>${fmtDT(ticket.createdTs)}</b></span>
                <span class="sep">·</span>
                <span>处理人 <b>${ticket.assignee ? esc(ticket.assignee) : '未分配'}</b></span>
                <span class="sep">·</span>
                <span>${esc(ticket.room)} · ${esc(ticket.device)}</span>
              </div>
            </div>
            <div class="dh-actions">${actionButtons(ticket)}</div>
          </section>

          <div class="detail-grid">
            <div class="detail-col">
              <section class="panel">
                <div class="panel-head">
                  <span class="panel-title"><svg class="ico"><use href="#i-info"/></svg>问题描述</span>
                  <span class="panel-extra">来自 ${ticket.source === 'auto' ? '自动告警' : '人工上报'}</span>
                </div>
                <div class="panel-body">
                  <p class="desc-text">${esc(ticket.description)}</p>
                  <div class="signal-box ${ticket.source === 'auto' ? 'is-auto' : 'is-manual'}">
                    <span class="signal-ico"><svg class="ico"><use href="#${ticket.source === 'auto' ? 'i-robot' : 'i-user'}"/></svg></span>
                    <div class="signal-main">
                      <div class="signal-title">
                        ${esc(ticket.signal.label)}
                        <span class="badge ${ticket.source === 'auto' ? 'tone-accent' : 'tone-slate'}">${ticket.source === 'auto' ? '信号触发' : '人工'}</span>
                      </div>
                      <div class="signal-ref">${esc(ticket.signal.ref)} · ${fmtDT(ticket.signal.at)}</div>
                    </div>
                  </div>
                </div>
              </section>

              <section class="panel">
                <div class="panel-head">
                  <span class="panel-title"><svg class="ico"><use href="#i-clock"/></svg>处理时间线 <span class="panel-title-count">${ticket.timeline.length} 条 · ${commentCount} 条评论</span></span>
                  <span class="panel-extra">按时间正序</span>
                </div>
                <div class="timeline">${timelineHtml(ticket)}</div>
                ${
                  canComment
                    ? `<div class="comment-box">
                        <textarea id="comment-input" placeholder="补充处理说明或评论；支持 ⌘ / Ctrl + Enter 提交"></textarea>
                        <div class="comment-foot">
                          <span class="comment-actor"><span class="avatar-xs is-me">张伟</span>以 张伟 身份评论</span>
                          <button class="btn btn-primary btn-sm" type="button" data-action="comment" data-id="${ticket.id}">
                            <svg class="ico"><use href="#i-send"/></svg>提交评论
                          </button>
                        </div>
                      </div>`
                    : `<div class="comment-locked"><svg class="ico"><use href="#i-info"/></svg>工单已${statusMeta.label}，如需继续跟进请先重开工单。</div>`
                }
              </section>
            </div>

            <div class="detail-col">
              <section class="panel">
                <div class="panel-head">
                  <span class="panel-title"><svg class="ico"><use href="#i-ticket"/></svg>工单信息</span>
                </div>
                <div class="panel-body">
                  <div class="meta-list">
                    <div class="meta-row"><span class="meta-k">工单编号</span><span class="meta-v mono">${esc(ticket.code)}</span></div>
                    <div class="meta-row"><span class="meta-k">状态</span><span class="meta-v">${statusBadge(ticket.status)}</span></div>
                    <div class="meta-row"><span class="meta-k">优先级</span><span class="meta-v">${priorityCell(ticket.priority)}</span></div>
                    <div class="meta-row"><span class="meta-k">分类</span><span class="meta-v">${categoryCell(ticket.category)}</span></div>
                    <div class="meta-row"><span class="meta-k">来源</span><span class="meta-v">${sourceBadge(ticket.source)}</span></div>
                    <div class="meta-row"><span class="meta-k">上报人</span><span class="meta-v">${esc(ticket.reporter)}</span></div>
                    <div class="meta-row"><span class="meta-k">处理人</span><span class="meta-v">${ticket.assignee ? personCell(ticket.assignee) : '<span class="person-name is-empty">未分配</span>'}</span></div>
                    <div class="meta-row"><span class="meta-k">创建时间</span><span class="meta-v mono">${fmtDT(ticket.createdTs)}</span></div>
                    <div class="meta-row"><span class="meta-k">更新时间</span><span class="meta-v mono">${fmtDT(ticket.updatedTs)}</span></div>
                    <div class="meta-row"><span class="meta-k">解决时间</span><span class="meta-v mono">${ticket.resolvedTs ? fmtDT(ticket.resolvedTs) : '—'}</span></div>
                    <div class="meta-row"><span class="meta-k">SLA 时限</span><span class="meta-v mono">${fmtDT(ticket.dueTs)}</span></div>
                  </div>
                </div>
              </section>

              <section class="panel">
                <div class="panel-head">
                  <span class="panel-title"><svg class="ico"><use href="#i-link"/></svg>关联上下文</span>
                  <span class="panel-extra">来自机器遥测与在线事件</span>
                </div>
                <div class="panel-body">
                  <div class="context-rows">
                    <div class="context-row">
                      <span class="context-ico"><svg class="ico"><use href="#i-room"/></svg></span>
                      <div class="context-main">
                        <div class="context-k">房间</div>
                        <div class="context-v">${esc(ticket.room)} ${esc(room.name)}</div>
                        <div class="context-sub">${esc(ticket.area)} · 设备：${esc(ticket.device)}</div>
                      </div>
                    </div>
                    <div class="context-row">
                      <span class="context-ico"><svg class="ico"><use href="#i-cpu"/></svg></span>
                      <div class="context-main">
                        <div class="context-k">机器节点</div>
                        <div class="context-v">${esc(ticket.hostname)} <span class="mono">${esc(ticket.serial)}</span></div>
                        <div class="context-sub">最近上报 ${fmtDT(ticket.createdTs - 3 * MIN)}</div>
                        <div class="context-states">
                          <span class="context-state">机器健康 <span class="state-chip ${health.tone}"><span class="dot"></span>${health.label}</span></span>
                          <span class="context-state">设备 <span class="state-chip ${online.tone}"><span class="dot"></span>${online.label}</span></span>
                        </div>
                      </div>
                    </div>
                  </div>
                  <div class="context-foot">
                    <span class="context-note">上下文为告警触发时的快照，非实时状态</span>
                    <button class="link-btn" type="button" data-action="machine-detail">
                      查看机器详情<svg class="ico"><use href="#i-arrow-right"/></svg>
                    </button>
                  </div>
                </div>
              </section>

              <section class="panel">
                <div class="panel-head">
                  <span class="panel-title"><svg class="ico"><use href="#i-alert"/></svg>SLA 时限</span>
                  <span class="panel-extra">${PRIORITY[ticket.priority].label}优先级</span>
                </div>
                <div class="panel-body">${slaHtml(ticket)}</div>
              </section>

              <section class="panel">
                <div class="panel-head">
                  <span class="panel-title"><svg class="ico"><use href="#i-bell"/></svg>来源与关联</span>
                </div>
                <div class="panel-body">
                  <div class="rel-list">
                    <div class="rel-row">
                      <span class="rel-ico"><svg class="ico"><use href="#i-trigger"/></svg></span>
                      <div class="rel-main">
                        <div class="rel-k">触发规则</div>
                        <div class="rel-v">${esc(ticket.signal.label)}</div>
                      </div>
                    </div>
                    <div class="rel-row">
                      <span class="rel-ico"><svg class="ico"><use href="#i-cpu"/></svg></span>
                      <div class="rel-main">
                        <div class="rel-k">机器序列号</div>
                        <div class="rel-v mono">${esc(ticket.serial)}</div>
                      </div>
                    </div>
                    <div class="rel-row">
                      <span class="rel-ico"><svg class="ico"><use href="#i-link"/></svg></span>
                      <div class="rel-main">
                        <div class="rel-k">关联快照 / 事件</div>
                        <div class="rel-v mono">${esc(ticket.signal.ref)}</div>
                      </div>
                    </div>
                    <div class="rel-row">
                      <span class="rel-ico"><svg class="ico"><use href="#i-room"/></svg></span>
                      <div class="rel-main">
                        <div class="rel-k">房间 / 区域</div>
                        <div class="rel-v">${esc(ticket.room)} · ${esc(ticket.area)}</div>
                      </div>
                    </div>
                  </div>
                </div>
              </section>
            </div>
          </div>
        </div>
      </div>`;

    if (keepScroll && oldScroll) {
      const body = $('.page-body');
      if (body) body.scrollTop = oldScroll;
    }
  }

  /* ------------------------------ 状态流转 ------------------------------ */

  const TRANSITIONS = {
    claim: { label: '认领工单', toast: '已认领' },
    start: { label: '开始处理', toast: '已开始处理' },
    resolve: { label: '标记为已解决', toast: '已标记解决' },
    close: { label: '关闭工单', toast: '已关闭' },
    reopen: { label: '重开工单', toast: '已重开' },
    cancel: { label: '取消工单', toast: '已取消' },
  };

  function applyAction(action, ticketId) {
    const ticket = findTicket(ticketId);
    if (!ticket) return;
    const ts = actionNow();
    let text = '';

    if (action === 'claim') {
      if (ticket.status !== 'open') return;
      if (ticket.assignee === ME) {
        toast('该工单已由你认领', 'warn');
        return;
      }
      ticket.assignee = ME;
      text = ticket.seed.assignee ? `从 ${ticket.seed.assignee} 接手处理。` : '认领后开始处理。';
    } else if (action === 'start') {
      if (ticket.status !== 'open') return;
      if (!ticket.assignee) ticket.assignee = ME;
      ticket.status = 'in_progress';
      text = `处理人：${ticket.assignee}。`;
    } else if (action === 'resolve') {
      if (ticket.status !== 'in_progress') return;
      ticket.status = 'resolved';
      ticket.resolvedTs = ts;
      text = '问题已处理，等待关闭确认。';
    } else if (action === 'close') {
      if (ticket.status !== 'resolved') return;
      ticket.status = 'closed';
      text = '确认问题已解决，关闭工单。';
    } else if (action === 'reopen') {
      if (ticket.status !== 'resolved' && ticket.status !== 'closed' && ticket.status !== 'cancelled') {
        return;
      }
      ticket.status = 'open';
      ticket.resolvedTs = null;
      ticket.assignee = ticket.assignee || ME;
      text = '问题复现或验证未通过，重新打开工单。';
    } else if (action === 'cancel') {
      if (ticket.status !== 'open' && ticket.status !== 'in_progress') return;
      ticket.status = 'cancelled';
      text = '经确认无需处理，取消工单。';
    } else {
      return;
    }

    ticket.updatedTs = ts;
    ticket.timeline.push({
      at: ts,
      actor: action === 'cancel' && ticket.status === 'cancelled' ? ME : action === 'reopen' ? ME : ticket.assignee || ME,
      kind:
        action === 'resolve' || action === 'close'
          ? 'resolved'
          : action === 'reopen'
            ? 'created'
            : 'status',
      action: TRANSITIONS[action].label,
      text,
      system: false,
    });
    ticket.timeline.sort((a, b) => a.at - b.at);

    toast(`${TRANSITIONS[action].toast}：${ticket.code}`);
    renderDetail(ticket.id, true);
  }

  function submitComment(ticketId) {
    const ticket = findTicket(ticketId);
    if (!ticket) return;
    if (ticket.status === 'closed' || ticket.status === 'cancelled') {
      toast('工单已关闭，无法继续评论', 'warn');
      return;
    }
    const input = $('#comment-input');
    const text = input ? input.value.trim() : '';
    if (!text) {
      toast('请输入评论内容', 'warn');
      if (input) input.focus();
      return;
    }
    const ts = actionNow();
    ticket.timeline.push({
      at: ts,
      actor: ME,
      kind: 'comment',
      action: '发表评论',
      text,
      system: false,
    });
    ticket.timeline.sort((a, b) => a.at - b.at);
    ticket.updatedTs = ts;
    toast('评论已提交');
    renderDetail(ticket.id, true);
  }

  /* ------------------------------ 新建工单弹窗 --------------------------- */

  function machineLabel(roomCode) {
    const room = roomOf(roomCode);
    return `${room.serial} · ${roomCode}-node`;
  }

  function openCreateModal() {
    const layer = $('#modal-layer');
    if (!layer) return;
    const roomOptions = ROOMS.map(
      (room) => `<option value="${room.code}">${room.code} · ${room.name}</option>`
    ).join('');
    layer.innerHTML = `
      <div class="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title">
        <div class="modal-head">
          <div>
            <div class="modal-title" id="modal-title">新建工单</div>
            <div class="modal-sub">手动上报问题；提交后进入「待处理」，并打开工单详情。</div>
          </div>
          <button class="icon-btn" type="button" data-modal-close title="关闭">
            <svg class="ico"><use href="#i-x"/></svg>
          </button>
        </div>
        <form id="new-ticket-form" novalidate>
          <div class="modal-body">
            <div class="field full">
              <label for="nt-title">标题<span class="req">*</span></label>
              <input id="nt-title" name="title" maxlength="80" autocomplete="off" placeholder="例如：room-103 投影仪无法开机" />
            </div>
            <div class="field">
              <label for="nt-category">分类</label>
              <select id="nt-category">
                ${CATEGORY_ORDER.map(
                  (key) => `<option value="${key}"${key === 'machine_health' ? ' selected' : ''}>${CATEGORY[key].label}</option>`
                ).join('')}
              </select>
            </div>
            <div class="field">
              <label for="nt-priority">优先级</label>
              <select id="nt-priority">
                ${PRIORITY_ORDER.map(
                  (key) => `<option value="${key}"${key === 'normal' ? ' selected' : ''}>${PRIORITY[key].label}（SLA ${PRIORITY[key].sla}h）</option>`
                ).join('')}
              </select>
            </div>
            <div class="field">
              <label for="nt-room">关联房间</label>
              <select id="nt-room">${roomOptions}</select>
            </div>
            <div class="field">
              <label for="nt-device">关联设备</label>
              <input id="nt-device" autocomplete="off" placeholder="默认使用房间主设备" />
            </div>
            <div class="field full">
              <label for="nt-machine">关联机器</label>
              <input id="nt-machine" readonly value="${esc(machineLabel('room-101'))}" />
              <span class="field-hint">机器信息由房间节点自动带出，与机器遥测页一致。</span>
            </div>
            <div class="field full">
              <label for="nt-desc">描述</label>
              <textarea id="nt-desc" placeholder="补充现象、影响范围、复现步骤等"></textarea>
            </div>
            <div class="form-warn" id="nt-warn">
              <svg class="ico"><use href="#i-alert"/></svg><span>请填写工单标题。</span>
            </div>
          </div>
          <div class="modal-foot">
            <span class="modal-foot-note">原型演示：提交后数据只写入浏览器内存，刷新即还原。</span>
            <span class="modal-foot-actions">
              <button class="btn btn-outline" type="button" data-modal-close>取消</button>
              <button class="btn btn-primary" type="submit">
                <svg class="ico"><use href="#i-plus"/></svg>创建工单
              </button>
            </span>
          </div>
        </form>
      </div>`;
    layer.hidden = false;
    const titleInput = $('#nt-title');
    if (titleInput) setTimeout(() => titleInput.focus(), 30);

    const roomSelect = $('#nt-room');
    const machineInput = $('#nt-machine');
    if (roomSelect && machineInput) {
      roomSelect.addEventListener('change', () => {
        machineInput.value = machineLabel(roomSelect.value);
      });
    }

    const form = $('#new-ticket-form');
    if (form) {
      form.addEventListener('submit', (event) => {
        event.preventDefault();
        const title = $('#nt-title').value.trim();
        if (!title) {
          $('#nt-warn').classList.add('is-on');
          $('#nt-title').focus();
          return;
        }
        const roomCode = $('#nt-room').value;
        const room = roomOf(roomCode);
        const deviceInput = $('#nt-device');
        const device = deviceInput.value.trim() || room.device;
        const ticket = createTicket({
          title,
          description: $('#nt-desc').value.trim(),
          category: $('#nt-category').value,
          priority: $('#nt-priority').value,
          room: roomCode,
          device,
        });
        closeModal();
        toast(`工单 ${ticket.code} 已创建`);
        if (location.hash === `#detail/${ticket.id}`) {
          renderDetail(ticket.id);
        } else {
          location.hash = `#detail/${ticket.id}`;
        }
      });
    }
  }

  function closeModal() {
    const layer = $('#modal-layer');
    if (!layer) return;
    layer.hidden = true;
    layer.innerHTML = '';
  }

  function nextCode(createdTs) {
    const prefix = `T-${fmtDateCompact(createdTs)}-`;
    let max = 0;
    tickets.forEach((ticket) => {
      if (ticket.code.startsWith(prefix)) {
        max = Math.max(max, Number(ticket.code.slice(prefix.length)) || 0);
      }
    });
    return `${prefix}${pad4(max + 1)}`;
  }

  function createTicket({ title, description, category, priority, room, device }) {
    const createdTs = actionNow();
    const roomInfo = roomOf(room);
    const signal = {
      kind: 'manual',
      label: '人工上报',
      ref: `manual_${fmtDateCompact(createdTs)}_${fmtTime(createdTs).replace(':', '')}`,
      at: createdTs,
    };
    ticketSeq += 1;
    const ticket = {
      id: `t${pad2(ticketSeq)}`,
      code: nextCode(createdTs),
      title,
      description: description || '（未填写补充描述）',
      status: 'open',
      priority,
      category,
      source: 'manual',
      assignee: null,
      reporter: ME,
      room,
      device: device || roomInfo.device,
      serial: roomInfo.serial,
      hostname: `${room}-node`,
      area: roomInfo.area,
      signal,
      createdTs,
      updatedTs: createdTs,
      resolvedTs: null,
      dueTs: createdTs + PRIORITY[priority].sla * HOUR,
      timeline: [],
      resolution: '',
      seed: { assignee: null, reporter: ME, source: 'manual' },
    };
    ticket.timeline.push({
      at: createdTs,
      actor: ME,
      kind: 'created',
      action: '创建工单',
      text: '人工上报工单',
      system: false,
    });
    tickets.push(ticket);
    ticketById.set(ticket.id, ticket);
    return ticket;
  }

  /* ------------------------------ 工单分析 ------------------------------ */

  function rangeWindow(days) {
    return {
      start: dayStart(NOW - (days - 1) * DAY),
      end: NOW,
    };
  }

  function createdInRange(days) {
    const win = rangeWindow(days);
    return tickets.filter(
      (ticket) => ticket.createdTs >= win.start && ticket.createdTs <= win.end
    );
  }

  function resolvedInRange(days) {
    const win = rangeWindow(days);
    return tickets.filter(
      (ticket) => ticket.resolvedTs && ticket.resolvedTs >= win.start && ticket.resolvedTs <= win.end
    );
  }

  function avgHandleMs(list) {
    if (!list.length) return 0;
    return list.reduce((sum, ticket) => sum + (ticket.resolvedTs - ticket.createdTs), 0) / list.length;
  }

  function deltaCountHtml(current, previous) {
    if (previous === 0) return '<span class="delta is-flat">较上期无数据</span>';
    const diff = current - previous;
    if (diff === 0) return '<span class="delta is-flat">与上期持平</span>';
    const cls = diff > 0 ? 'is-up' : 'is-down';
    const pct = ` (${diff > 0 ? '+' : ''}${Math.round((diff / previous) * 100)}%)`;
    return `<span class="delta ${cls}">${diff > 0 ? '↑' : '↓'} ${Math.abs(diff)}${pct} 较上期</span>`;
  }

  function bucketsFor(days) {
    const win = rangeWindow(days);
    const buckets = [];
    for (let i = 0; i < days; i += 1) {
      buckets.push({ start: win.start + i * DAY, created: 0, resolved: 0 });
    }
    const indexOf = (ts) => Math.round((dayStart(ts) - win.start) / DAY);
    tickets.forEach((ticket) => {
      const createdIndex = indexOf(ticket.createdTs);
      if (createdIndex >= 0 && createdIndex < days) buckets[createdIndex].created += 1;
      if (ticket.resolvedTs) {
        const resolvedIndex = indexOf(ticket.resolvedTs);
        if (resolvedIndex >= 0 && resolvedIndex < days) buckets[resolvedIndex].resolved += 1;
      }
    });
    return buckets;
  }

  function chartHtml(days) {
    const buckets = bucketsFor(days);
    const maxValue = Math.max(1, ...buckets.map((bucket) => Math.max(bucket.created, bucket.resolved)));
    const axisMax = Math.max(2, Math.ceil(maxValue * 1.15));
    const labelStep = days <= 7 ? 1 : days <= 30 ? 5 : 14;
    const labelEvery = (index) =>
      index % labelStep === 0 || index === buckets.length - 1 ? fmtMD(buckets[index].start) : '';

    const gridLines = [0, 1, 2, 3]
      .map((index) => {
        const top = (index / 3) * 100;
        return `<i style="top:${top}%"></i>`;
      })
      .join('');

    const axisLabels = [axisMax, Math.round((axisMax * 2) / 3), Math.round(axisMax / 3), 0]
      .map((value) => `<span>${value}</span>`)
      .join('');

    const bars = buckets
      .map((bucket) => {
        const newHeight = bucket.created ? Math.max(5, (bucket.created / axisMax) * 100) : 0;
        const closedHeight = bucket.resolved ? Math.max(5, (bucket.resolved / axisMax) * 100) : 0;
        return `<div class="bar-group" title="${fmtDate(bucket.start)}：新增 ${bucket.created} · 关闭 ${bucket.resolved}">
          <span class="bar" style="height:${newHeight.toFixed(1)}%"></span>
          <span class="bar is-closed" style="height:${closedHeight.toFixed(1)}%"></span>
        </div>`;
      })
      .join('');

    const totalCreated = buckets.reduce((sum, bucket) => sum + bucket.created, 0);
    const totalResolved = buckets.reduce((sum, bucket) => sum + bucket.resolved, 0);
    const net = totalCreated - totalResolved;

    return `
      <div class="chart-body">
        <div class="chart-axis">${axisLabels}</div>
        <div class="chart-plot">
          <div class="chart-grid">${gridLines}</div>
          <div class="chart-bars${days > 30 ? ' is-dense' : ''}">${bars}</div>
        </div>
      </div>
      <div class="chart-x">${buckets.map((bucket, index) => `<span>${labelEvery(index)}</span>`).join('')}</div>
      <div class="chart-foot">
        <span>新增 <b>${totalCreated}</b> 张 · 关闭 <b>${totalResolved}</b> 张 · 净增 <b>${net >= 0 ? '+' : ''}${net}</b></span>
        <span>悬停柱体查看当日明细</span>
      </div>`;
  }

  function categoryBarsHtml(list) {
    const counts = CATEGORY_ORDER.map((key) => ({
      key,
      count: list.filter((ticket) => ticket.category === key).length,
    })).sort((a, b) => b.count - a.count);
    const max = Math.max(1, ...counts.map((item) => item.count));
    return counts
      .map(
        (item) => `
        <div class="hbar-row">
          <span class="hbar-label" title="${CATEGORY[item.key].label}">${CATEGORY[item.key].label}</span>
          <span class="hbar-track"><span class="hbar-fill" style="width:${(item.count / max) * 100}%;background:${CATEGORY[item.key].color}"></span></span>
          <span class="hbar-value">${item.count}</span>
        </div>`
      )
      .join('');
  }

  function roomsRankHtml(list) {
    const map = new Map();
    list.forEach((ticket) => {
      map.set(ticket.room, (map.get(ticket.room) || 0) + 1);
    });
    const rows = Array.from(map.entries())
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5);
    if (!rows.length) return '<div class="tl-empty">所选区间暂无数据</div>';
    return rows
      .map(
        ([code, count], index) => `
        <div class="rank-row">
          <span class="rank-no">${index + 1}</span>
          <span class="rank-main">
            <span class="rank-name">${esc(code)}</span>
            <span class="rank-sub">${esc(roomOf(code).name)} · ${esc(roomOf(code).area)}</span>
          </span>
          <span class="rank-count">${count}<small> 张</small></span>
        </div>`
      )
      .join('');
  }

  function devicesRankHtml(list) {
    const map = new Map();
    list.forEach((ticket) => {
      const key = ticket.device;
      const entry = map.get(key) || { count: 0, rooms: new Set() };
      entry.count += 1;
      entry.rooms.add(ticket.room);
      map.set(key, entry);
    });
    const rows = Array.from(map.entries())
      .sort((a, b) => b[1].count - a[1].count)
      .slice(0, 5);
    if (!rows.length) return '<div class="tl-empty">所选区间暂无数据</div>';
    return rows
      .map(
        ([device, entry], index) => `
        <div class="rank-row">
          <span class="rank-no">${index + 1}</span>
          <span class="rank-main">
            <span class="rank-name">${esc(device)}</span>
            <span class="rank-sub">涉及 ${entry.rooms.size} 个房间</span>
          </span>
          <span class="rank-count">${entry.count}<small> 张</small></span>
        </div>`
      )
      .join('');
  }

  function loadHtml(list) {
    const map = new Map();
    list.forEach((ticket) => {
      const key = ticket.assignee || '未分配';
      const entry = map.get(key) || {
        open: 0,
        in_progress: 0,
        resolved: 0,
        closed: 0,
        cancelled: 0,
        total: 0,
      };
      entry[ticket.status] += 1;
      entry.total += 1;
      map.set(key, entry);
    });
    const order = [...HANDLERS, '未分配'];
    const rows = order
      .map((name) => ({ name, data: map.get(name) || { open: 0, in_progress: 0, resolved: 0, closed: 0, cancelled: 0, total: 0 } }))
      .filter((row) => row.data.total > 0);
    const maxTotal = Math.max(1, ...rows.map((row) => row.data.total));

    return rows
      .map((row) => {
        const segs = [];
        [
          ['open', 'is-open', '待处理'],
          ['in_progress', 'is-progress', '处理中'],
          ['resolved', 'is-resolved', '已解决'],
          ['closed', 'is-closed', '已关闭'],
          ['cancelled', 'is-cancelled', '已取消'],
        ].forEach(([key, cls, label]) => {
          if (row.data[key]) {
            segs.push(
              `<span class="load-seg ${cls}" style="width:${(row.data[key] / maxTotal) * 100}%" title="${label} ${row.data[key]}"></span>`
            );
          }
        });
        return `
          <div class="load-row">
            <span class="load-name">${row.name === '未分配' ? '<span class="avatar-xs">—</span>未分配' : `<span class="avatar-xs${row.name === ME ? ' is-me' : ''}">${esc(row.name.slice(0, 1))}</span>${esc(row.name)}`}</span>
            <span class="load-track">${segs.join('')}</span>
            <span class="load-total">${row.data.total}</span>
          </div>`;
      })
      .join('');
  }

  function insightNoteHtml(days, list) {
    const roomCategory = new Map();
    list.forEach((ticket) => {
      const key = `${ticket.room}|${ticket.category}`;
      roomCategory.set(key, (roomCategory.get(key) || 0) + 1);
    });
    let topPair = null;
    roomCategory.forEach((count, key) => {
      if (!topPair || count > topPair.count) {
        const [room, category] = key.split('|');
        topPair = { room, category, count };
      }
    });
    const roomCounts = new Map();
    list.forEach((ticket) => roomCounts.set(ticket.room, (roomCounts.get(ticket.room) || 0) + 1));
    let topRoom = null;
    roomCounts.forEach((count, room) => {
      if (!topRoom || count > topRoom.count) topRoom = { room, count };
    });

    const categoryCounts = CATEGORY_ORDER.map((key) => ({
      key,
      count: list.filter((ticket) => ticket.category === key).length,
    })).sort((a, b) => b.count - a.count);
    const topCategory = categoryCounts[0];

    if (!list.length) {
      return {
        title: '重复故障观察',
        html: `近 ${days} 天暂无工单数据，无法判断故障聚集情况。`,
      };
    }

    if (topPair && topPair.count >= 2) {
      const info = roomOf(topPair.room);
      const roomTotal = roomCounts.get(topPair.room) || topPair.count;
      return {
        title: '重复故障 / 高频告警提醒',
        html: `近 <b>${days}</b> 天 <b>${topPair.room}</b>（${esc(info.name)}）产生 <b>${roomTotal}</b> 张工单，其中 <b>${topPair.count}</b> 张为${CATEGORY[topPair.category].label}类，同一节点反复触发；建议合并同类告警或调整阈值。${
          topCategory
            ? `全部新增中 ${CATEGORY[topCategory.key].label}类占 <b>${Math.round((topCategory.count / list.length) * 100)}%</b>（${topCategory.count} 张），为最高频来源。`
            : ''
        }`,
      };
    }

    return {
      title: '重复故障 / 高频告警观察',
      html: `近 <b>${days}</b> 天工单较分散，暂无同房间同类故障重复聚集；${topRoom ? `当前告警密度最高的是 <b>${topRoom.room}</b>（${topRoom.count} 张），` : ''}建议持续观察 ${
        topCategory ? `${CATEGORY[topCategory.key].label}类（${topCategory.count} 张）` : '告警分布'
      }的变化。`,
    };
  }

  function renderInsights() {
    const days = state.insightsDays;
    const win = rangeWindow(days);
    const prevWin = { start: win.start - days * DAY, end: win.start };
    const created = createdInRange(days);
    const resolved = resolvedInRange(days);
    const prevCreated = tickets.filter(
      (ticket) => ticket.createdTs >= prevWin.start && ticket.createdTs < prevWin.end
    );
    const prevResolved = tickets.filter(
      (ticket) => ticket.resolvedTs && ticket.resolvedTs >= prevWin.start && ticket.resolvedTs < prevWin.end
    );

    const autoCount = created.filter((ticket) => ticket.source === 'auto').length;
    const manualCount = created.length - autoCount;
    const unresolved = created.filter(
      (ticket) => ticket.status !== 'closed' && ticket.status !== 'cancelled'
    ).length;
    const inProgress = created.filter((ticket) => ticket.status === 'in_progress').length;
    const overdue = created.filter(isOverdue).length;
    const avg = avgHandleMs(resolved);
    const prevAvg = avgHandleMs(prevResolved);
    const avgParts = fmtDurationParts(avg);
    const resolveRate = created.length ? Math.round((resolved.length / created.length) * 100) : 0;
    const overdueRate = created.length ? Math.round((overdue / created.length) * 100) : 0;

    let avgDelta = '<span class="delta is-flat">较上期 —</span>';
    if (resolved.length && prevResolved.length) {
      const diffHours = (avg - prevAvg) / HOUR;
      const cls = diffHours < 0 ? 'is-up' : diffHours > 0 ? 'is-down' : 'is-flat';
      avgDelta = `<span class="delta ${cls}">${diffHours < 0 ? '↓' : diffHours > 0 ? '↑' : ''} ${Math.abs(diffHours).toFixed(1)}h 较上期</span>`;
    }

    const topHandled = resolved.length
      ? (() => {
          const map = new Map();
          resolved.forEach((ticket) => {
            const by = ticket.assignee || '未分配';
            map.set(by, (map.get(by) || 0) + 1);
          });
          return Array.from(map.entries()).sort((a, b) => b[1] - a[1])[0];
        })()
      : null;

    const kpis = [
      {
        tone: 'accent',
        icon: 'i-ticket',
        label: '新增工单',
        value: created.length,
        unit: '张',
        foot: `${deltaCountHtml(created.length, prevCreated.length)}`,
      },
      {
        tone: 'ok',
        icon: 'i-check-circle',
        label: '已解决',
        value: resolved.length,
        unit: '张',
        foot: `解决率 <b>${resolveRate}%</b> · ${deltaCountHtml(resolved.length, prevResolved.length)}`,
      },
      {
        tone: 'amber',
        icon: 'i-inbox',
        label: '未关闭',
        value: unresolved,
        unit: '张',
        foot: `处理中 <b>${inProgress}</b> 张${topHandled ? ` · ${esc(topHandled[0])} 解决最多` : ''}`,
      },
      {
        tone: 'accent',
        icon: 'i-clock',
        label: '平均处理时长',
        value: resolved.length ? avgParts.value : '—',
        unit: resolved.length ? avgParts.unit : '',
        foot: avgDelta,
      },
      {
        tone: 'danger',
        icon: 'i-alert',
        label: '超时工单',
        value: overdue,
        unit: '张',
        foot: `占新增 <b>${overdueRate}%</b> · SLA 已过期`,
      },
    ];

    const note = insightNoteHtml(days, created);

    $('#page').innerHTML = `
      ${pageHead(
        '工单分析',
        '按时间窗口统计工单新增、关闭与 SLA 表现，定位高频故障房间与处理负载。',
        `<div class="ins-range" role="tablist" aria-label="统计时间范围">
          ${[7, 30, 90]
            .map(
              (item) =>
                `<button type="button" class="${state.insightsDays === item ? 'is-active' : ''}" data-action="ins-range" data-days="${item}">近 ${item} 天</button>`
            )
            .join('')}
        </div>
        <button class="btn btn-outline" type="button" data-action="export">
          <svg class="ico"><use href="#i-audit"/></svg><span>导出报表</span>
        </button>`
      )}
      <div class="page-body">
        <div class="page-body-inner">
          <div class="ins-kpis">
            ${kpis
              .map(
                (kpi) => `
              <div class="ins-kpi is-${kpi.tone}">
                <div class="ins-kpi-top">
                  <span class="ins-kpi-label">${kpi.label}</span>
                  <span class="ins-kpi-ico"><svg class="ico"><use href="#${kpi.icon}"/></svg></span>
                </div>
                <div class="ins-kpi-value">${kpi.value}${kpi.unit ? `<small> ${kpi.unit}</small>` : ''}</div>
                <div class="ins-kpi-foot">${kpi.foot}</div>
              </div>`
              )
              .join('')}
          </div>

          <div class="ins-row ins-row-2">
            <section class="panel chart-card">
              <div class="panel-head">
                <span class="panel-title"><svg class="ico"><use href="#i-chart"/></svg>新增 vs 关闭（按日）</span>
                <span class="chart-legend">
                  <span><i class="legend-dot"></i>新增</span>
                  <span><i class="legend-dot is-closed"></i>关闭</span>
                </span>
              </div>
              <div class="panel-body">${chartHtml(days)}</div>
            </section>

            <section class="panel">
              <div class="panel-head">
                <span class="panel-title"><svg class="ico"><use href="#i-filter"/></svg>分类分布</span>
                <span class="panel-extra">近 ${days} 天新增</span>
              </div>
              <div class="panel-body">
                <div class="hbar-list">${categoryBarsHtml(created)}</div>
                <div class="chart-foot">
                  <span>共 <b>${created.length}</b> 张工单</span>
                  <span>按分类占比排序</span>
                </div>
              </div>
            </section>
          </div>

          <div class="ins-row ins-row-3">
            <section class="panel">
              <div class="panel-head">
                <span class="panel-title"><svg class="ico"><use href="#i-room"/></svg>房间 TOP5</span>
                <span class="panel-extra">工单量</span>
              </div>
              <div class="panel-body"><div class="rank-list">${roomsRankHtml(created)}</div></div>
            </section>

            <section class="panel">
              <div class="panel-head">
                <span class="panel-title"><svg class="ico"><use href="#i-device"/></svg>设备 TOP5</span>
                <span class="panel-extra">工单量</span>
              </div>
              <div class="panel-body"><div class="rank-list">${devicesRankHtml(created)}</div></div>
            </section>

            <section class="panel">
              <div class="panel-head">
                <span class="panel-title"><svg class="ico"><use href="#i-users"/></svg>处理人负载</span>
                <span class="panel-extra">按状态堆叠</span>
              </div>
              <div class="panel-body">
                <div class="load-list">${loadHtml(created)}</div>
                <div class="load-legend">
                  <span><i class="legend-dot" style="background:#dc6803"></i>待处理</span>
                  <span><i class="legend-dot" style="background:#3a5bd9"></i>处理中</span>
                  <span><i class="legend-dot" style="background:#0f9d58"></i>已解决</span>
                  <span><i class="legend-dot" style="background:#c2c9d6"></i>已关闭</span>
                </div>
              </div>
            </section>
          </div>

          <div class="insight-note">
            <span class="insight-ico"><svg class="ico"><use href="#i-alert"/></svg></span>
            <div class="insight-main">
              <div class="insight-title">${note.title}</div>
              <div class="insight-text">${note.html}</div>
            </div>
          </div>
        </div>
      </div>`;
  }

  /* ------------------------------ 路由 / 侧栏 --------------------------- */

  function parseRoute() {
    const raw = (location.hash || '#list').replace(/^#\/?/, '');
    if (!raw || raw === 'list') return { view: 'list' };
    if (raw === 'insights') return { view: 'insights' };
    if (raw.startsWith('detail/')) return { view: 'detail', id: raw.slice('detail/'.length) };
    return { view: 'list' };
  }

  function updateChrome(route) {
    const crumb = $('#crumb');
    if (crumb) {
      if (route.view === 'detail') {
        const ticket = findTicket(route.id);
        crumb.innerHTML = `<span class="crumb-parent">控制台</span><span class="crumb-sep">/</span><span class="crumb-mid">工单</span><span class="crumb-sep">/</span><span class="crumb-cur">${esc(ticket ? ticket.code : '详情')}</span>`;
      } else if (route.view === 'insights') {
        crumb.innerHTML = `<span class="crumb-parent">控制台</span><span class="crumb-sep">/</span><span class="crumb-cur">工单分析</span>`;
      } else {
        crumb.innerHTML = `<span class="crumb-parent">控制台</span><span class="crumb-sep">/</span><span class="crumb-cur">工单</span>`;
      }
    }
    const navTickets = $('#nav-tickets');
    const navInsights = $('#nav-insights');
    if (navTickets) navTickets.classList.toggle('is-active', route.view === 'list' || route.view === 'detail');
    if (navInsights) navInsights.classList.toggle('is-active', route.view === 'insights');
    const hash = $('#sb-hash');
    if (hash) hash.textContent = route.view === 'insights' ? 'ticket-insights' : 'tickets';
  }

  function render() {
    const route = parseRoute();
    if (route.view === 'detail') {
      if (!findTicket(route.id)) {
        location.hash = '#list';
        return;
      }
      renderDetail(route.id);
    } else if (route.view === 'insights') {
      renderInsights();
    } else {
      renderList();
    }
    updateChrome(route);
    const body = $('.page-body');
    if (body) body.scrollTop = 0;
  }

  /* ------------------------------ 事件绑定 ------------------------------ */

  function bindEvents() {
    const page = $('#page');

    page.addEventListener('click', (event) => {
      const target = event.target.closest('[data-action]');
      if (!target) return;
      const action = target.dataset.action;

      if (action === 'goto-detail') {
        const id = target.dataset.id;
        if (id) location.hash = `#detail/${id}`;
        return;
      }
      if (action === 'tab') {
        state.status = target.dataset.status;
        state.page = 1;
        refreshList();
        return;
      }
      if (action === 'page') {
        const next = Number(target.dataset.page);
        if (!Number.isFinite(next)) return;
        state.page = next;
        refreshList();
        return;
      }
      if (action === 'reset') {
        resetFilters();
        renderList();
        return;
      }
      if (action === 'new') {
        openCreateModal();
        return;
      }
      if (action === 'refresh') {
        toast('已刷新工单数据（静态 mock）');
        return;
      }
      if (action === 'machine-detail') {
        toast('原型未包含机器详情页；正式实现将跳转 /machines/{serial}');
        return;
      }
      if (action === 'export') {
        toast('原型演示：导出报表只保留入口，不生成文件');
        return;
      }
      if (action === 'ins-range') {
        state.insightsDays = Number(target.dataset.days) || 30;
        renderInsights();
        updateChrome(parseRoute());
        return;
      }
      if (['claim', 'start', 'resolve', 'close', 'reopen', 'cancel'].includes(action)) {
        applyAction(action, target.dataset.id || parseRoute().id);
        return;
      }
      if (action === 'comment') {
        submitComment(target.dataset.id || parseRoute().id);
        return;
      }
    });

    page.addEventListener('change', (event) => {
      const field = event.target.closest('[data-filter]');
      if (!field) return;
      const key = field.dataset.filter;
      state[key] = field.value;
      state.page = 1;
      refreshList();
    });

    page.addEventListener('input', (event) => {
      if (event.target.id !== 'list-search') return;
      state.search = event.target.value.trim();
      state.page = 1;
      refreshList();
    });

    // 全局搜索：回车把关键词带入列表搜索
    const globalSearch = $('#global-search');
    if (globalSearch) {
      globalSearch.addEventListener('keydown', (event) => {
        if (event.key !== 'Enter') return;
        state.search = globalSearch.value.trim();
        state.page = 1;
        if (parseRoute().view === 'list') {
          renderList();
        } else {
          location.hash = '#list';
        }
      });
    }

    // 侧边栏 / 顶栏中不属于原型的入口：不跳转，给一句说明
    document.addEventListener('click', (event) => {
      const other = event.target.closest('.nav-other, .topbar-link[data-page]');
      if (!other) return;
      event.preventDefault();
      toast(`「${other.dataset.page}」在既有控制台已实现；本原型只演示工单模块`);
    });

    // 弹层
    const layer = $('#modal-layer');
    if (layer) {
      layer.addEventListener('click', (event) => {
        if (event.target === layer || event.target.closest('[data-modal-close]')) closeModal();
      });
    }

    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') {
        const layer = $('#modal-layer');
        if (layer && !layer.hidden) closeModal();
        return;
      }
      const input = event.target;
      if (input && input.id === 'comment-input' && (event.metaKey || event.ctrlKey) && event.key === 'Enter') {
        event.preventDefault();
        submitComment(parseRoute().id);
      }
    });

    window.addEventListener('hashchange', () => {
      closeModal();
      render();
    });
  }

  /* ------------------------------ 启动 --------------------------------- */

  function init() {
    bindEvents();
    render();
    const clock = $('#sb-clock');
    if (clock) clock.textContent = fmtTime(NOW);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
