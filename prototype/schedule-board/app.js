/* ==========================================================================
   Room Node Console · 跨房间排课原型（纯前端静态 demo）
   --------------------------------------------------------------------------
   - 完全离线：不请求任何外部资源
   - 三视图共享同一套 mock 课块，仅投影不同：
       A 周网格课表 / B 资源时间轴 / C 全房间编排矩阵
   - URL hash: #week | #timeline | #matrix（可带课块 id，如 #timeline/b05 直接
     打开详情抽屉，方便截图与分享定位）
   ========================================================================== */
(function () {
  'use strict';

  /* ------------------------------ 小工具 ------------------------------ */
  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));
  const pad2 = (n) => String(n).padStart(2, '0');
  const clamp = (n, lo, hi) => Math.min(hi, Math.max(lo, n));
  const snap = (n, step = 10) => Math.round(n / step) * step;
  const overlap = (a1, a2, b1, b2) => a1 < b2 && b1 < a2;

  function mix(hex, target, t) {
    const p = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
    const [r1, g1, b1] = p(hex);
    const [r2, g2, b2] = p(target);
    const c = (x, y) => pad2(Math.round(x + (y - x) * t).toString(16));
    return '#' + c(r1, r2) + c(g1, g2) + c(b1, b2);
  }

  const fmtTime = (m) => `${pad2(Math.floor(m / 60))}:${pad2(m % 60)}`;
  const fmtRange = (s, e) => `${fmtTime(s)}–${fmtTime(e)}`;

  /* ------------------------------ Mock 数据 ------------------------------ */
  const BASE_YEAR = 2026;
  const BASE_MONTH = 10; // 11 月
  const BASE_DAY = 2; // 2026-11-02 周一（第 45 周）

  const DAYS = [
    { i: 1, name: '周一' },
    { i: 2, name: '周二' },
    { i: 3, name: '周三' },
    { i: 4, name: '周四' },
    { i: 5, name: '周五' },
    { i: 6, name: '周六' },
    { i: 7, name: '周日' },
  ];

  const PERIODS = [
    { idx: 1, label: '第1-2节', start: 480, end: 580 },
    { idx: 2, label: '第3-4节', start: 600, end: 700 },
    { idx: 3, label: '第5-6节', start: 810, end: 910 },
    { idx: 4, label: '第7-8节', start: 930, end: 1030 },
    { idx: 5, label: '第9-10节', start: 1110, end: 1210 },
  ];

  const DAY_START = 480; // 08:00
  const DAY_END = 1260; // 21:00
  const DAY_SPAN = DAY_END - DAY_START;

  const ROOMS = [
    { code: 'P3-101', name: '智慧教室 A', area: 'P3', capacity: 50, device: 'room-501' },
    { code: 'P3-102', name: '研讨教室 B', area: 'P3', capacity: 40 },
    { code: 'P3-201', name: '阶梯教室', area: 'P3', capacity: 60 },
    { code: 'P3-202', name: '计算机房 C', area: 'P3', capacity: 48 },
    { code: 'P4-101', name: '多媒体教室', area: 'P4', capacity: 45 },
    { code: 'P4-102', name: '语音教室', area: 'P4', capacity: 40 },
    { code: 'P4-301', name: '合班教室', area: 'P4', capacity: 60 },
    { code: 'P4-302', name: '创新工坊', area: 'P4', capacity: 42 },
  ];

  const COURSES = [
    { id: 'c1', name: '高等数学', teacher: '张伟', cls: '计科2301', c: '#4a6fd4' },
    { id: 'c2', name: '大学物理', teacher: '李娜', cls: '计科2301', c: '#3d8f9b' },
    { id: 'c3', name: '程序设计基础', teacher: '王强', cls: '软工2302', c: '#7b6bc4' },
    { id: 'c4', name: '数据结构', teacher: '陈静', cls: '计科2302', c: '#4f9a72' },
    { id: 'c5', name: '计算机网络', teacher: '刘洋', cls: '网工2301', c: '#c98a3c' },
    { id: 'c6', name: '操作系统', teacher: '赵敏', cls: '计科2302', c: '#b8618a' },
    { id: 'c7', name: '大学英语', teacher: '周慧', cls: '软工2302', c: '#5a9a8e' },
    { id: 'c8', name: '体育', teacher: '吴磊', cls: '计科2301', c: '#77839a' },
  ];

  const DRAFT_COURSE = {
    id: 'draft',
    name: '未命名课程',
    teacher: '待定',
    cls: '—',
    c: '#8b94a3',
    draft: true,
  };

  [...COURSES, DRAFT_COURSE].forEach((c) => {
    c.soft = mix(c.c, '#ffffff', 0.91);
    c.edge = mix(c.c, '#ffffff', 0.68);
  });

  const COURSE_MAP = new Map([...COURSES, DRAFT_COURSE].map((c) => [c.id, c]));
  const courseOf = (id) => COURSE_MAP.get(id) || DRAFT_COURSE;
  const roomOf = (code) => ROOMS.find((r) => r.code === code);
  const areaOfRoom = (code) => (roomOf(code) || {}).area || '';

  // 34 个课块：周一 14 节（含 2 处冲突）、周二/周三各 5、周四/周五各 4、周末各 1
  const seedBlocks = [
    // 周一 2026-11-02
    [1, 480, 580, 'P3-101', 'c1'],
    [1, 480, 580, 'P3-201', 'c2'],
    [1, 480, 580, 'P4-101', 'c3'],
    [1, 480, 580, 'P3-202', 'c7'],
    [1, 600, 700, 'P3-102', 'c4'],
    [1, 660, 740, 'P3-102', 'c6', true], // 同房间冲突（单日覆盖）
    [1, 600, 700, 'P4-102', 'c5'],
    [1, 810, 910, 'P4-302', 'c8'],
    [1, 810, 910, 'P3-201', 'c7'],
    [1, 810, 910, 'P3-101', 'c1'],
    [1, 830, 950, 'P4-102', 'c1', true], // 同教师冲突（调课，与上一节张伟重叠）
    [1, 930, 1030, 'P4-301', 'c4'],
    [1, 1110, 1210, 'P3-202', 'c6'],
    [1, 1110, 1210, 'P4-101', 'c5'],
    // 周二
    [2, 480, 580, 'P3-101', 'c3'],
    [2, 600, 700, 'P4-102', 'c7'],
    [2, 810, 910, 'P3-202', 'c4'],
    [2, 930, 1030, 'P4-301', 'c5'],
    [2, 1110, 1210, 'P4-302', 'c8'],
    // 周三
    [3, 480, 580, 'P3-102', 'c1'],
    [3, 600, 700, 'P4-101', 'c6'],
    [3, 810, 910, 'P3-201', 'c2'],
    [3, 930, 1030, 'P3-202', 'c3'],
    [3, 1110, 1210, 'P4-301', 'c7'],
    // 周四
    [4, 480, 580, 'P3-101', 'c4'],
    [4, 810, 910, 'P4-102', 'c5'],
    [4, 930, 1030, 'P3-102', 'c1'],
    [4, 1110, 1210, 'P4-302', 'c8'],
    // 周五
    [5, 480, 580, 'P3-201', 'c2'],
    [5, 600, 700, 'P3-102', 'c6'],
    [5, 810, 910, 'P4-101', 'c3'],
    [5, 930, 1030, 'P4-301', 'c8'],
    // 周六 / 周日
    [6, 600, 700, 'P3-101', 'c7'],
    [7, 1110, 1210, 'P3-202', 'c1'],
  ];

  let seq = 0;
  const nextId = () => `b${pad2(++seq)}`;
  let BLOCKS = seedBlocks.map(([day, start, end, room, course, override]) => ({
    id: nextId(),
    day,
    start,
    end,
    room,
    course,
    override: !!override,
    draft: false,
  }));

  /* ------------------------------ 状态 ------------------------------ */
  const state = {
    view: 'week',
    weekOffset: 0,
    day: 1, // 时间轴当前日
    area: 'all',
    room: 'all', // 仅周网格使用的房间筛选（各视图独立记忆）
    search: '',
    conflictFocus: false,
    batchMode: false, // 仅编排矩阵
    selectedId: null,
    drawerOpen: false,
    matrixSel: null, // {r1,r2,c1,c2}
  };

  let CONFLICTS = { pairs: [], byBlock: new Map() };

  /* ------------------------------ 日期 ------------------------------ */
  function dateOf(day) {
    const d = new Date(BASE_YEAR, BASE_MONTH, BASE_DAY);
    d.setDate(d.getDate() + state.weekOffset * 7 + (day - 1));
    return d;
  }
  const fmtMD = (d) => `${pad2(d.getMonth() + 1)}/${pad2(d.getDate())}`;
  const fmtISO = (d) =>
    `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
  const weekdayName = (day) => DAYS[day - 1].name;

  function weekLabel() {
    const s = dateOf(1);
    const e = dateOf(7);
    return `${BASE_YEAR} 第 ${45 + state.weekOffset} 周 · ${fmtMD(s)}–${fmtMD(e)}`;
  }
  function dayLabel() {
    return `${fmtMD(dateOf(state.day))} ${weekdayName(state.day)}`;
  }

  /* --------------------------- 冲突 / 过滤 --------------------------- */
  function recomputeConflicts() {
    const pairs = [];
    const byBlock = new Map();
    const list = BLOCKS.filter((b) => !b.draft);
    for (let i = 0; i < list.length; i++) {
      for (let j = i + 1; j < list.length; j++) {
        const a = list[i];
        const b = list[j];
        if (a.day !== b.day) continue;
        if (!overlap(a.start, a.end, b.start, b.end)) continue;
        const sameRoom = a.room === b.room;
        const ca = courseOf(a.course);
        const cb = courseOf(b.course);
        const sameTeacher = ca.teacher && cb.teacher && ca.teacher === cb.teacher;
        if (!sameRoom && !sameTeacher) continue;
        const type = sameRoom ? 'room' : 'teacher';
        pairs.push({ type, aId: a.id, bId: b.id });
        const push = (id, otherId) => {
          if (!byBlock.has(id)) byBlock.set(id, []);
          byBlock.get(id).push({ type, otherId });
        };
        push(a.id, b.id);
        push(b.id, a.id);
      }
    }
    CONFLICTS = { pairs, byBlock };
  }

  const conflictOf = (id) => CONFLICTS.byBlock.get(id) || [];
  const isConflict = (id) => CONFLICTS.byBlock.has(id);
  // 冲突类型：同房间 = room（红色）；同教师 = teacher（琥珀色）
  function conflictTypeOf(id) {
    const list = conflictOf(id);
    if (!list.length) return null;
    return list.some((c) => c.type === 'room') ? 'room' : 'teacher';
  }
  const conflictClass = (id) =>
    conflictTypeOf(id) === 'room'
      ? ' is-conflict'
      : conflictTypeOf(id) === 'teacher'
        ? ' is-conflict is-teacher'
        : '';

  function blockVisible(b) {
    if (state.area !== 'all' && areaOfRoom(b.room) !== state.area) return false;
    if (state.view === 'week' && state.room !== 'all' && b.room !== state.room) return false;
    return true;
  }

  function searchHidden(b) {
    if (state.conflictFocus && !isConflict(b.id)) return true;
    const q = state.search.trim().toLowerCase();
    if (!q) return false;
    const co = courseOf(b.course);
    return !`${co.name} ${co.teacher} ${co.cls} ${b.room}`.toLowerCase().includes(q);
  }

  function periodOf(b) {
    return PERIODS.find((p) => b.start >= p.start && b.start < p.end) || PERIODS[0];
  }

  function filteredRooms() {
    return ROOMS.filter((r) => state.area === 'all' || r.area === state.area);
  }

  /* ------------------------------ Toast ------------------------------ */
  let toastTimer = null;
  function toast(msg, kind) {
    const wrap = $('#toast-wrap');
    const el = document.createElement('div');
    el.className = `toast${kind === 'warn' ? ' is-warn' : ''}`;
    el.innerHTML = `<svg class="ico"><use href="#${
      kind === 'warn' ? 'i-alert' : 'i-check'
    }" /></svg><span>${msg}</span>`;
    wrap.appendChild(el);
    while (wrap.children.length > 3) wrap.removeChild(wrap.firstElementChild);
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      el.classList.add('is-out');
      setTimeout(() => el.remove(), 200);
    }, 2800);
  }

  /* ------------------------------ 图例 ------------------------------ */
  const legendItemsHtml = () =>
    COURSES.map(
      (c) =>
        `<span class="legend-item"><i style="background:${c.c}"></i>${c.name}</span>`,
    ).join('');
  function renderLegend() {
    $('#legend-overlay').innerHTML = legendItemsHtml();
  }

  /* ------------------------------ 周网格 ------------------------------ */
  function blockTitleAttr(b) {
    const co = courseOf(b.course);
    const ct = conflictTypeOf(b.id);
    return `${co.name} · ${co.teacher} · ${co.cls} · ${b.room} · ${fmtRange(b.start, b.end)}${
      b.override ? ' · 单日覆盖' : ''
    }${ct === 'room' ? ' · 房间冲突' : ct === 'teacher' ? ' · 教师冲突' : ''}`;
  }

  function weekBlockHtml(b) {
    const co = courseOf(b.course);
    const conf = isConflict(b.id);
    const dim = searchHidden(b);
    const sel = state.selectedId === b.id;
    return `<button type="button" class="blk${conflictClass(b.id)}${
      dim ? ' is-dim' : ''
    }${sel ? ' is-selected' : ''}${b.draft ? ' is-draft' : ''}" data-block="${b.id}"
      style="--c:${co.c};--soft:${co.soft};--edge:${co.edge}" title="${blockTitleAttr(b)}">
      <span class="blk-title">
        ${conf ? '<svg class="blk-warn"><use href="#i-alert" /></svg>' : ''}
        <span class="grow">${co.name}</span>
        ${b.override ? '<span class="blk-tag">调</span>' : ''}
      </span>
      <span class="blk-sub"><span class="rm">${b.room}</span><span class="dot-sep">·</span><span>${co.teacher}</span></span>
    </button>`;
  }

  function renderWeek() {
    const el = $('#view-week');
    const byCell = new Map();
    BLOCKS.filter(blockVisible).forEach((b) => {
      const p = periodOf(b);
      const key = `${b.day}-${p.idx}`;
      if (!byCell.has(key)) byCell.set(key, []);
      byCell.get(key).push(b);
    });

    let html = '<div class="week-grid">';
    html += '<div class="wk-head wk-corner">节次 / 时间</div>';
    DAYS.forEach((d) => {
      html += `<div class="wk-head${d.i >= 6 ? ' is-weekend' : ''}"><span>${d.name}</span><span class="wk-date">${fmtMD(dateOf(d.i))}</span></div>`;
    });

    PERIODS.forEach((p) => {
      html += `<div class="wk-period"><span class="p-idx">${p.label}</span><span class="p-time">${fmtTime(p.start)}–${fmtTime(p.end)}</span></div>`;
      DAYS.forEach((d) => {
        const list = (byCell.get(`${d.i}-${p.idx}`) || []).slice().sort((a, b) => {
          const ca = isConflict(a.id) ? 0 : 1;
          const cb = isConflict(b.id) ? 0 : 1;
          if (ca !== cb) return ca - cb;
          return a.room.localeCompare(b.room) || a.start - b.start;
        });
        const shown = list.slice(0, 3);
        const rest = list.length - shown.length;
        html += `<div class="wk-cell${d.i >= 6 ? ' is-weekend' : ''}">${shown
          .map(weekBlockHtml)
          .join('')}${
          rest > 0
            ? `<button type="button" class="wk-more" title="另外 ${rest} 个课块">+${rest}</button>`
            : ''
        }</div>`;
      });
    });
    html += '</div>';
    html += `<div class="view-footer">
      <div class="legend-row">${legendItemsHtml()}</div>
      <div class="vf-hint">
        <span class="swatch"><i style="background:var(--danger)"></i>房间重叠</span>
        <span class="swatch"><i style="background:var(--amber)"></i>教师重叠</span>
        <span>· 每格最多完整展示 3 个课块，超出显示 +N</span>
      </div>
    </div>`;
    el.innerHTML = html;
  }

  /* ------------------------------ 时间轴 ------------------------------ */
  function tlBlockHtml(b, lane) {
    const co = courseOf(b.course);
    const ctype = conflictTypeOf(b.id);
    const dim = searchHidden(b);
    const sel = state.selectedId === b.id;
    const left = ((b.start - DAY_START) / DAY_SPAN) * 100;
    const width = ((b.end - b.start) / DAY_SPAN) * 100;
    const i = lane ? lane.i : 0;
    // 同房间重叠：最早课块占满行高，后续块向下错位 26px 只露标题，避免互相压字
    const pos = i === 0 ? 'top:5px;bottom:5px;' : `top:${31 + (i - 1) * 22}px;bottom:5px;`;
    const tag =
      ctype && i === 0
        ? `<span class="tl-tag is-${ctype}">${ctype === 'room' ? '房间冲突' : '教师冲突'}</span>`
        : '';
    return `<div class="tl-block${conflictClass(b.id)}${i > 0 ? ' is-lane' : ''}${
      dim ? ' is-dim' : ''
    }${sel ? ' is-selected' : ''}${b.draft ? ' is-draft' : ''}" data-block="${b.id}"
      style="left:${left.toFixed(3)}%;width:${width.toFixed(3)}%;${pos}z-index:${3 + i};--c:${co.c};--soft:${co.soft};--edge:${co.edge}"
      title="${blockTitleAttr(b)}">
      <span class="tb-title"><span class="tb-name">${co.name}</span>${tag}</span>
      <span class="tb-sub">${co.teacher} · ${co.cls}</span>
      <span class="tb-time">${fmtRange(b.start, b.end)}</span>
      ${b.override ? '<span class="tl-ov">调</span>' : ''}
      <span class="tl-handle is-left"></span><span class="tl-handle is-right"></span>
    </div>`;
  }

  function laneRow(blocks) {
    const sorted = blocks.slice().sort((a, b) => a.start - b.start || a.end - b.end);
    const groups = [];
    let cur = null;
    sorted.forEach((b) => {
      if (!cur || b.start >= cur.maxEnd) {
        cur = { maxEnd: b.end, items: [b] };
        groups.push(cur);
      } else {
        cur.items.push(b);
        cur.maxEnd = Math.max(cur.maxEnd, b.end);
      }
    });
    const res = new Map();
    groups.forEach((g) => {
      g.items.forEach((b, i) => res.set(b.id, { i, n: g.items.length }));
    });
    return res;
  }

  function renderTimeline() {
    const el = $('#view-timeline');
    const rooms = filteredRooms();
    const dayBlocks = BLOCKS.filter((b) => b.day === state.day && blockVisible(b) && !searchHidden(b));
    const shownCount = BLOCKS.filter((b) => b.day === state.day && blockVisible(b)).length;

    let html = '<div class="tl">';
    // 时间刻度
    html += '<div class="tl-ruler"><div class="tl-ruler-head">房间 · 时间</div><div class="tl-ruler-track">';
    for (let i = 0; i <= 13; i++) {
      const cls = i === 0 ? ' is-first' : i === 13 ? ' is-last' : '';
      html += `<span class="tl-hour${cls}" style="left:${((i / 13) * 100).toFixed(4)}%">${fmtTime(
        DAY_START + i * 60,
      )}</span>`;
    }
    html += '</div></div><div class="tl-body">';

    ['P3', 'P4'].forEach((area) => {
      const group = rooms.filter((r) => r.area === area);
      if (!group.length) return;
      const count = group.reduce(
        (n, r) => n + BLOCKS.filter((b) => b.day === state.day && b.room === r.code).length,
        0,
      );
      html += `<div class="tl-group"><div class="tl-area"><span class="bar"></span>${area} 区域<span class="cnt">${group.length} 间教室 · 今日 ${count} 节</span></div><div class="tl-rows">`;
      group.forEach((room, ri) => {
        const rowBlocks = dayBlocks.filter((b) => b.room === room.code);
        const lanes = laneRow(rowBlocks);
        html += `<div class="tl-row${ri % 2 === 1 ? ' is-alt' : ''}" data-room="${room.code}">`;
        html += `<div class="tl-room"><span class="line1"><span class="tl-room-code">${room.code}</span><span class="tl-room-cap">${room.capacity} 座</span></span><span class="tl-room-name">${room.name}${room.device ? ' · ' + room.device : ''}</span></div>`;
        html += `<div class="tl-track" data-room="${room.code}">`;
        rowBlocks.forEach((b) => {
          html += tlBlockHtml(b, lanes.get(b.id));
        });
        html += '</div></div>';
      });
      html += '</div></div>';
    });

    html += '</div></div>';
    el.innerHTML = html;
    if (!rooms.length) {
      el.innerHTML = '<div class="tl"><div class="tl-ruler"><div class="tl-ruler-head">房间 · 时间</div><div class="tl-ruler-track"></div></div><div class="tl-empty">当前筛选下没有房间</div></div>';
    }
    el.dataset.count = String(shownCount);
  }

  /* ------------------------------ 编排矩阵 ------------------------------ */
  function mxBlockHtml(b) {
    const co = courseOf(b.course);
    const ctype = conflictTypeOf(b.id);
    const dim = searchHidden(b);
    const sel = state.selectedId === b.id;
    return `<button type="button" class="mx-block${conflictClass(b.id)}${
      dim ? ' is-dim' : ''
    }${sel ? ' is-selected' : ''}${b.draft ? ' is-draft' : ''}" data-block="${b.id}"
      style="--c:${co.c};--soft:${co.soft};--edge:${co.edge}" title="${blockTitleAttr(b)}">
      <span class="mb-title"><span class="mb-name">${co.name}</span>${
        b.override ? '<span class="mb-tag">调</span>' : ''
      }</span>
      <span class="mb-sub">${co.teacher}${
        ctype === 'room' ? ' · 房间冲突' : ctype === 'teacher' ? ' · 教师冲突' : ''
      }</span>
    </button>`;
  }

  function renderMatrix() {
    const el = $('#view-matrix');
    const rooms = filteredRooms();
    const byCell = new Map();
    BLOCKS.filter(blockVisible).forEach((b) => {
      if (b.day < 1 || b.day > 7) return;
      const p = periodOf(b);
      const key = `${b.room}-${b.day}-${p.idx}`;
      if (!byCell.has(key)) byCell.set(key, []);
      byCell.get(key).push(b);
    });

    let html = '<div class="mx"><div class="mx-scroll"><div class="mx-grid" style="grid-template-rows:28px 30px repeat(';
    html += `${Math.max(rooms.length, 1)},minmax(62px,1fr))">`;
    html += `<div class="mx-corner"><span class="t1">房间 / 时段</span><span class="t2">周一–周日 · 5 个节次</span></div>`;

    DAYS.forEach((d, di) => {
      html += `<div class="mx-day${d.i >= 6 ? ' is-weekend' : ''}" style="grid-row:1;grid-column:${
        2 + di * 5
      } / span 5">${d.name}<span class="d">${fmtMD(dateOf(d.i))}</span></div>`;
    });

    DAYS.forEach((d) => {
      PERIODS.forEach((p, pi) => {
        const cls = [
          'mx-ph',
          pi === 0 ? 'is-daystart' : '',
          d.i >= 6 ? 'is-weekend' : '',
        ].filter(Boolean).join(' ');
        html += `<div class="${cls}" style="grid-row:2;grid-column:${
          2 + (d.i - 1) * 5 + pi
        }"><span class="p1">${p.label}</span><span class="p2">${fmtTime(p.start)}</span></div>`;
      });
    });

    rooms.forEach((room, ri) => {
      const rowIndex = ri + 3;
      html += `<div class="mx-room" style="grid-row:${rowIndex};grid-column:1"><span class="r1"><span class="rcode">${room.code}</span><span class="rcap">${room.capacity} 座</span></span><span class="rname">${room.name}${
        room.device ? ' · ' + room.device : ''
      }</span></div>`;
      DAYS.forEach((d) => {
        PERIODS.forEach((p, pi) => {
          const col = (d.i - 1) * 5 + pi; // 0..34
          const list = (byCell.get(`${room.code}-${d.i}-${p.idx}`) || []).slice().sort((a, b) => {
            const ca = isConflict(a.id) ? 0 : 1;
            const cb = isConflict(b.id) ? 0 : 1;
            return ca - cb || a.start - b.start;
          });
          const cellCls = [
            'mx-cell',
            pi === 0 ? 'is-daystart' : '',
            d.i >= 6 ? 'is-weekend' : '',
            list.length ? 'is-filled' : '',
          ].filter(Boolean).join(' ');
          html += `<div class="${cellCls}" data-room="${room.code}" data-day="${d.i}" data-pid="${
            p.idx
          }" data-row="${ri}" data-col="${col}" style="grid-row:${rowIndex};grid-column:${
            2 + col
          }">${list.map(mxBlockHtml).join('')}${
            list.length ? '' : '<span class="mx-plus"><svg class="ico"><use href="#i-plus" /></svg></span>'
          }</div>`;
        });
      });
    });

    html += '</div></div>';
    html += `<div class="mx-footer"><div class="legend-row">${legendItemsHtml()}</div><span class="mx-hint"><svg class="ico" style="width:12px;height:12px"><use href="#i-target" /></svg>拖拽框选多个时段批量应用 · 横向滚动查看周三–周日</span></div>`;
    html += '</div>';
    el.innerHTML = html;

    // 让 11 个完整列落在视口内（1440 下 ≈ 88.9px/列），右边缘不出现半截课块
    const scrollEl = $('#view-matrix .mx-scroll');
    const grid = $('#view-matrix .mx-grid');
    if (scrollEl && grid) {
      const avail = scrollEl.clientWidth - 156;
      const colW = avail > 400 ? Math.floor((avail / 11) * 100) / 100 : 88;
      grid.style.gridTemplateColumns = `156px repeat(35, ${Math.max(74, colW)}px)`;
    }

    applyMatrixSelection();
  }

  function applyMatrixSelection() {
    const mx = $('#view-matrix .mx');
    if (!mx) return;
    $$('.mx-cell', mx).forEach((cell) => cell.classList.remove('is-selected'));
    const old = $('.batch-bar', mx);
    if (old) old.remove();
    const sel = state.matrixSel;
    if (!sel) return;
    const { r1, r2, c1, c2 } = sel;
    $$('.mx-cell', mx).forEach((cell) => {
      const r = +cell.dataset.row;
      const c = +cell.dataset.col;
      if (r >= r1 && r <= r2 && c >= c1 && c <= c2) cell.classList.add('is-selected');
    });
    const count = (r2 - r1 + 1) * (c2 - c1 + 1);
    const bar = document.createElement('div');
    bar.className = 'batch-bar';
    bar.innerHTML = `<span class="count">已选 <b>${count}</b> 个时段</span><span class="vsep"></span>
      <button type="button" class="btn btn-apply" data-batch="apply">批量应用课程到 ${count} 个时段</button>
      <button type="button" class="btn btn-ghost" data-batch="clear">清除</button>`;
    mx.appendChild(bar);
  }

  /* ------------------------------ 工具条 ------------------------------ */
  function renderToolbar() {
    $('#week-label').textContent = weekLabel();
    $('#day-label').textContent = dayLabel();
    $('#nav-week').hidden = state.view === 'timeline';
    $('#nav-day').hidden = state.view !== 'timeline';
    $('#field-room').style.display = state.view === 'week' ? '' : 'none';
    $('#batch-switch').hidden = state.view !== 'matrix';
    $('#sel-area').value = state.area;
    $('#sel-room').value = state.room;
    $('#input-search').value = state.search;
    $('#input-batch').checked = state.batchMode;

    const n = CONFLICTS.pairs.length;
    const chip = $('#btn-conflict');
    chip.querySelector('b').textContent = String(n);
    chip.classList.toggle('is-clear', n === 0);
    chip.classList.toggle('is-active', state.conflictFocus && n > 0);
    chip.title =
      n > 0
        ? `发现 ${n} 处冲突（同房间 / 同教师）· 点击聚焦`
        : '当前无冲突';
  }

  /* ------------------------------ 抽屉 ------------------------------ */
  function renderDrawer() {
    const drawer = $('#drawer');
    const b = BLOCKS.find((x) => x.id === state.selectedId);
    if (!state.drawerOpen || !b) {
      drawer.classList.remove('is-open');
      drawer.setAttribute('aria-hidden', 'true');
      drawer.innerHTML = '';
      return;
    }
    const co = courseOf(b.course);
    const room = roomOf(b.room);
    const cons = conflictOf(b.id);

    const conflictHtml = cons
      .map((c) => {
        const other = BLOCKS.find((x) => x.id === c.otherId);
        if (!other) return '';
        const oc = courseOf(other.course);
        return `<div class="conflict-card${c.type === 'teacher' ? ' is-teacher' : ''}">
          <div class="cc-head"><svg class="ico"><use href="#i-alert" /></svg>${
            c.type === 'room' ? '房间冲突 · 同房间时段重叠' : '教师冲突 · 同一教师时段重叠'
          }</div>
          <div class="cc-text">与「${oc.name}」在 ${other.room} ${fmtRange(
          other.start,
          other.end,
        )} 重叠，排课前需要调整其中一个时段。</div>
          <button type="button" class="cc-other" data-select-block="${other.id}">
            <span>${oc.name} · ${oc.teacher} · ${other.room}</span>
            <span class="r">${weekdayName(other.day)} ${fmtRange(other.start, other.end)}</span>
          </button>
        </div>`;
      })
      .join('');

    drawer.innerHTML = `<div class="drawer-head"><span class="t">课程详情</span>
      <button type="button" class="icon-btn" data-dw="close" title="关闭"><svg class="ico"><use href="#i-x" /></svg></button></div>
      <div class="drawer-body">
        <div class="dw-hero">
          <span class="mark" style="--c:${co.c}"></span>
          <div>
            <div class="dw-name">${co.name}</div>
            <div class="dw-tags">
              <span class="badge ${b.override ? '' : 'is-accent'}">${
      b.override ? '单日覆盖' : '周模板'
    }</span>
              ${b.draft ? '<span class="badge">草稿</span>' : ''}
              ${
                cons.length
                  ? `<span class="badge ${
                      conflictTypeOf(b.id) === 'teacher' ? 'is-amber' : 'is-warn'
                    }"><svg class="ico" style="width:10px;height:10px"><use href="#i-alert" /></svg>${
                      conflictTypeOf(b.id) === 'teacher' ? '教师冲突' : '房间冲突'
                    } · ${cons.length}</span>`
                  : '<span class="badge">无冲突</span>'
              }
            </div>
          </div>
        </div>
        <div class="dw-section">
          <div class="dw-section-title">课程信息</div>
          <div class="dw-row"><span class="k">教师</span><span class="v">${co.teacher}</span></div>
          <div class="dw-row"><span class="k">班级</span><span class="v">${co.cls}</span></div>
          <div class="dw-row"><span class="k">房间</span><span class="v">${b.room} · ${room ? room.name : '—'}</span></div>
          <div class="dw-row"><span class="k">区域</span><span class="v">${areaOfRoom(b.room)} 区域</span></div>
          <div class="dw-row"><span class="k">容量</span><span class="v">${room ? room.capacity + ' 座' : '—'}</span></div>
          <div class="dw-row"><span class="k">设备</span><span class="v mono">${room && room.device ? room.device : '—'}</span></div>
        </div>
        <div class="dw-section">
          <div class="dw-section-title">时间</div>
          <div class="dw-row"><span class="k">日期</span><span class="v">${fmtISO(dateOf(b.day))} ${weekdayName(b.day)}</span></div>
          <div class="dw-row"><span class="k">时间</span><span class="v mono">${fmtRange(b.start, b.end)}</span></div>
          <div class="dw-row"><span class="k">时长</span><span class="v mono">${b.end - b.start} 分钟 · ${periodOf(b).label}</span></div>
        </div>
        ${conflictHtml}
        <div class="dw-note">${
          b.override
            ? `<b>单日覆盖</b>：仅覆盖 ${fmtMD(dateOf(b.day))}（${weekdayName(
                b.day,
              )}）这一次课，不改动周模板；删除后该日恢复模板排课。`
            : `<b>周模板</b>：每周${weekdayName(b.day).replace('周', '')} ${fmtRange(
                b.start,
                b.end,
              )} 自动生成排课实例并触发房间设备事件。`
        }</div>
      </div>
      <div class="drawer-foot">
        <button type="button" class="btn btn-outline" data-dw="edit"><svg class="ico"><use href="#i-edit" /></svg>编辑</button>
        <button type="button" class="btn btn-danger" data-dw="delete"><svg class="ico"><use href="#i-trash" /></svg>删除</button>
        <button type="button" class="btn btn-outline btn-with-caret" data-dw="copy"><svg class="ico"><use href="#i-copy" /></svg>复制到…</button>
      </div>`;
    drawer.classList.add('is-open');
    drawer.setAttribute('aria-hidden', 'false');
  }

  function openDrawer(id) {
    if (!BLOCKS.some((b) => b.id === id)) return;
    state.selectedId = id;
    state.drawerOpen = true;
    renderDrawer();
    renderWeek();
    renderTimeline();
    renderMatrix();
    setHash(`${state.view}/${id}`);
  }

  function closeDrawer() {
    if (!state.drawerOpen) return;
    state.drawerOpen = false;
    state.selectedId = null;
    renderDrawer();
    renderWeek();
    renderTimeline();
    renderMatrix();
    setHash(state.view);
  }

  /* ------------------------------ 弹层 ------------------------------ */
  function closePopover() {
    $$('.popover').forEach((p) => p.remove());
  }

  function openPopover(html, opts) {
    closePopover();
    const stage = $('#stage');
    const el = document.createElement('div');
    el.className = 'popover';
    el.innerHTML = html;
    stage.appendChild(el);
    const stageRect = stage.getBoundingClientRect();
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    let x = opts.x;
    let y = opts.y;
    if (opts.anchorEl) {
      const r = opts.anchorEl.getBoundingClientRect();
      x = r.left - stageRect.left + r.width / 2 - w / 2;
      y = r.top - stageRect.top - h - 8;
    }
    x = clamp(x, 8, stageRect.width - w - 8);
    y = clamp(y, 8, stageRect.height - h - 8);
    el.style.left = `${Math.round(x)}px`;
    el.style.top = `${Math.round(y)}px`;
    return el;
  }

  const courseOptionsHtml = (selected) =>
    COURSES.map(
      (c) => `<option value="${c.id}"${c.id === selected ? ' selected' : ''}>${c.name} · ${c.teacher}</option>`,
    ).join('');

  function openCreatePopover(roomCode, start, end, x, y) {
    const rooms = filteredRooms();
    const el = openPopover(
      `<div class="popover-head">新建课程<button type="button" class="icon-btn" data-pop="close"><svg class="ico"><use href="#i-x" /></svg></button></div>
       <div class="popover-body">
         <div class="popover-meta"><span>${roomCode}</span><span class="mono">${weekdayName(state.day)} ${fmtRange(start, end)}</span></div>
         <div class="field"><label>课程</label><select data-pop="course">${courseOptionsHtml(
           'c4',
         )}</select></div>
         <div class="popover-meta" data-pop="teacher">教师 陈静 · 班级 计科2302</div>
       </div>
       <div class="popover-foot"><button type="button" class="btn btn-ghost" data-pop="close">取消</button>
       <button type="button" class="btn btn-primary" data-pop="create">创建</button></div>`,
      { x, y },
    );
    const select = $('[data-pop="course"]', el);
    const meta = $('[data-pop="teacher"]', el);
    const sync = () => {
      const c = courseOf(select.value);
      meta.textContent = `教师 ${c.teacher} · 班级 ${c.cls}`;
    };
    select.addEventListener('change', sync);
    el.addEventListener('click', (e) => {
      if (e.target.closest('[data-pop="close"]')) closePopover();
      if (e.target.closest('[data-pop="create"]')) {
        const courseId = select.value;
        const co = courseOf(courseId);
        BLOCKS.push({
          id: nextId(),
          day: state.day,
          start,
          end,
          room: roomCode,
          course: courseId,
          override: true,
          draft: false,
        });
        closePopover();
        recomputeConflicts();
        render();
        toast(`已新建课程：${co.name} · ${weekdayName(state.day)} ${fmtRange(start, end)}`);
      }
    });
  }

  /* ------------------------------ 模态：新建课程 ------------------------------ */
  let modalState = null;

  function openNewModal(prefill) {
    const p = Object.assign(
      {
        course: 'c1',
        room: 'P3-101',
        day: 1,
        period: 1,
        source: 'template',
      },
      prefill || {},
    );
    modalState = p;
    const layer = $('#modal-layer');
    layer.hidden = false;
    layer.innerHTML = `<div class="modal" role="dialog" aria-modal="true" aria-label="新建课程">
      <div class="modal-head">
        <div><div class="modal-title">新建课程</div>
        <div class="modal-sub">演示原型：提交后仅在前端生效，不会写入后端。</div></div>
        <button type="button" class="icon-btn" data-modal="close"><svg class="ico"><use href="#i-x" /></svg></button>
      </div>
      <div class="modal-body">
        <div class="field"><label>课程</label><select data-m="course">${courseOptionsHtml(p.course)}</select></div>
        <div class="field"><label>教师 / 班级</label><div class="ro" data-m="tc">张伟 · 计科2301</div></div>
        <div class="field"><label>房间</label><select data-m="room">${ROOMS.map(
          (r) => `<option value="${r.code}"${r.code === p.room ? ' selected' : ''}>${r.code} · ${r.name}</option>`,
        ).join('')}</select></div>
        <div class="field"><label>星期</label><select data-m="day">${DAYS.map(
          (d) => `<option value="${d.i}"${d.i === p.day ? ' selected' : ''}>${d.name}</option>`,
        ).join('')}</select></div>
        <div class="field"><label>节次</label><select data-m="period">${PERIODS.map(
          (x) =>
            `<option value="${x.idx}"${x.idx === p.period ? ' selected' : ''}>${x.label} · ${fmtRange(
              x.start,
              x.end,
            )}</option>`,
        ).join('')}</select></div>
        <div class="field"><label>来源</label>
          <div class="source-seg"><button type="button" data-m="src" data-src="template" class="${
            p.source === 'template' ? 'is-active' : ''
          }">周模板</button><button type="button" data-m="src" data-src="override" class="${
      p.source === 'override' ? 'is-active' : ''
    }">单日覆盖</button></div>
        </div>
        <div class="form-warn" data-m="warn"><svg class="ico"><use href="#i-alert" /></svg><span></span></div>
      </div>
      <div class="modal-foot">
        <button type="button" class="btn btn-outline" data-modal="close">取消</button>
        <button type="button" class="btn btn-primary" data-modal="create">创建课程</button>
      </div>
    </div>`;
    syncModal();
    layer.querySelector('[data-m="course"]').addEventListener('change', syncModal);
    layer.querySelector('[data-m="room"]').addEventListener('change', syncModal);
    layer.querySelector('[data-m="day"]').addEventListener('change', syncModal);
    layer.querySelector('[data-m="period"]').addEventListener('change', syncModal);
    $$('[data-m="src"]', layer).forEach((btn) =>
      btn.addEventListener('click', () => {
        modalState.source = btn.dataset.src;
        $$('[data-m="src"]', layer).forEach((b) =>
          b.classList.toggle('is-active', b === btn),
        );
      }),
    );
    layer.addEventListener('click', onModalClick);
  }

  function syncModal() {
    const layer = $('#modal-layer');
    const courseSel = layer.querySelector('[data-m="course"]');
    const co = courseOf(courseSel.value);
    modalState.course = co.id;
    modalState.room = layer.querySelector('[data-m="room"]').value;
    modalState.day = +layer.querySelector('[data-m="day"]').value;
    modalState.period = +layer.querySelector('[data-m="period"]').value;
    layer.querySelector('[data-m="tc"]').textContent = `${co.teacher} · ${co.cls}`;

    const p = PERIODS.find((x) => x.idx === modalState.period);
    const hit = BLOCKS.find(
      (b) =>
        !b.draft &&
        b.day === modalState.day &&
        b.room === modalState.room &&
        overlap(b.start, b.end, p.start, p.end),
    );
    const warn = layer.querySelector('[data-m="warn"]');
    if (hit) {
      const hc = courseOf(hit.course);
      warn.classList.add('is-on');
      warn.querySelector('span').textContent = `该房间该时段已有「${hc.name}」${fmtRange(
        hit.start,
        hit.end,
      )}，创建后会标记为冲突。`;
    } else {
      warn.classList.remove('is-on');
    }
  }

  function closeModal() {
    const layer = $('#modal-layer');
    layer.hidden = true;
    layer.innerHTML = '';
    layer.removeEventListener('click', onModalClick);
    modalState = null;
  }

  function onModalClick(e) {
    const t = e.target.closest('[data-modal]');
    if (!t) {
      if (e.target.id === 'modal-layer') closeModal();
      return;
    }
    if (t.dataset.modal === 'close') return closeModal();
    if (t.dataset.modal === 'create') {
      const p = PERIODS.find((x) => x.idx === modalState.period);
      const co = courseOf(modalState.course);
      const dayName = weekdayName(modalState.day);
      BLOCKS.push({
        id: nextId(),
        day: modalState.day,
        start: p.start,
        end: p.end,
        room: modalState.room,
        course: modalState.course,
        override: modalState.source === 'override',
        draft: false,
      });
      closeModal();
      recomputeConflicts();
      render();
      toast(`已新建课程：${co.name} · ${dayName} ${fmtRange(p.start, p.end)}`);
    }
  }

  /* --------------------------- 时间轴：拖拽/新建 --------------------------- */
  let tlDrag = null;

  function trackPxPerMin(trackEl) {
    return trackEl.getBoundingClientRect().width / DAY_SPAN;
  }

  function hitAt(room, day, start, end, ignoreId) {
    return BLOCKS.filter(
      (b) =>
        b.id !== ignoreId &&
        !b.draft &&
        b.day === day &&
        b.room === room &&
        overlap(b.start, b.end, start, end),
    );
  }

  function teacherHit(courseId, day, start, end, ignoreId) {
    const co = courseOf(courseId);
    if (!co.teacher || co.teacher === '待定') return [];
    return BLOCKS.filter((b) => {
      if (b.id === ignoreId || b.draft || b.day !== day) return false;
      const bc = courseOf(b.course);
      return bc.teacher === co.teacher && overlap(b.start, b.end, start, end);
    });
  }

  function rowAtY(y) {
    const rows = $$('#view-timeline .tl-row');
    let best = null;
    let bestDist = Infinity;
    rows.forEach((r) => {
      const rect = r.getBoundingClientRect();
      const d = y < rect.top ? rect.top - y : y > rect.bottom ? y - rect.bottom : 0;
      if (d < bestDist) {
        bestDist = d;
        best = r;
      }
    });
    return best;
  }

  function startTimelineDrag(e, blockEl) {
    const id = blockEl.dataset.block;
    const b = BLOCKS.find((x) => x.id === id);
    if (!b) return;
    tlDrag = {
      id,
      blockEl,
      startX: e.clientX,
      startY: e.clientY,
      orig: { start: b.start, end: b.end, room: b.room },
      moved: false,
      preview: null,
      candidate: null,
      valid: true,
    };
    e.preventDefault();
    document.addEventListener('pointermove', onTimelineDragMove);
    document.addEventListener('pointerup', onTimelineDragUp, { once: true });
  }

  function onTimelineDragMove(e) {
    if (!tlDrag) return;
    const d = tlDrag;
    const dx = e.clientX - d.startX;
    const dy = e.clientY - d.startY;
    if (!d.moved && Math.abs(dx) + Math.abs(dy) < 5) return;
    d.moved = true;
    d.blockEl.classList.add('is-dragging');
    const row = rowAtY(e.clientY);
    if (!row) return;
    const track = row.querySelector('.tl-track');
    const ppm = trackPxPerMin(track);
    const dur = d.orig.end - d.orig.start;
    let start = snap(d.orig.start + dx / ppm, 10);
    start = clamp(start, DAY_START, DAY_END - dur);
    const room = row.dataset.room;

    if (!d.preview || d.preview.parentElement !== track) {
      if (d.preview) d.preview.remove();
      d.preview = document.createElement('div');
      d.preview.className = 'tl-preview';
      d.preview.innerHTML =
        '<span class="ok"></span><span class="bad">时段冲突 · 不可放置</span>';
      track.appendChild(d.preview);
    }
    const prev = d.preview;
    prev.style.left = `${(((start - DAY_START) / DAY_SPAN) * 100).toFixed(3)}%`;
    prev.style.width = `${((dur / DAY_SPAN) * 100).toFixed(3)}%`;
    prev.style.top = '5px';
    prev.style.bottom = '5px';

    const b = BLOCKS.find((x) => x.id === d.id);
    const roomHits = hitAt(room, b.day, start, start + dur, d.id);
    const tHits = teacherHit(b.course, b.day, start, start + dur, b.id);
    d.valid = roomHits.length === 0 && tHits.length === 0;
    d.hit = roomHits[0] || tHits[0] || null;
    prev.querySelector('.ok').textContent = `${fmtRange(start, start + dur)} · ${room}`;
    prev.classList.toggle('is-invalid', !d.valid);
    d.candidate = { start, end: start + dur, room };
  }

  function onTimelineDragUp() {
    document.removeEventListener('pointermove', onTimelineDragMove);
    if (!tlDrag) return;
    const d = tlDrag;
    tlDrag = null;
    d.blockEl.classList.remove('is-dragging');
    if (d.preview) d.preview.remove();
    if (!d.moved) {
      openDrawer(d.id);
      return;
    }
    if (!d.candidate) return;
    if (!d.valid) {
      toast(
        d.hit
          ? `与「${courseOf(d.hit.course).name}」${fmtRange(d.hit.start, d.hit.end)} 冲突，已取消移动`
          : '目标时段与已有课程冲突，已取消移动',
        'warn',
      );
      return;
    }
    const b = BLOCKS.find((x) => x.id === d.id);
    const changed =
      b.start !== d.candidate.start || b.end !== d.candidate.end || b.room !== d.candidate.room;
    if (!changed) return;
    b.start = d.candidate.start;
    b.end = d.candidate.end;
    b.room = d.candidate.room;
    recomputeConflicts();
    render();
    toast(
      `已移动「${courseOf(b.course).name}」到 ${b.room} ${weekdayName(b.day)} ${fmtRange(
        b.start,
        b.end,
      )}`,
    );
  }

  /* --------------------------- 矩阵：框选/批量 --------------------------- */
  let mxDrag = null;

  function cellBoundsFrom(a, b) {
    return {
      r1: Math.min(a.row, b.row),
      r2: Math.max(a.row, b.row),
      c1: Math.min(a.col, b.col),
      c2: Math.max(a.col, b.col),
    };
  }

  function updateMatrixSelection(bounds) {
    state.matrixSel = bounds;
    applyMatrixSelection();
  }

  function startMatrixDrag(e, cell) {
    mxDrag = {
      anchor: { row: +cell.dataset.row, col: +cell.dataset.col },
      moved: false,
      cell,
    };
    state.matrixSel = null;
    applyMatrixSelection();
    document.addEventListener('pointermove', onMatrixDragMove);
    document.addEventListener('pointerup', onMatrixDragUp, { once: true });
  }

  function onMatrixDragMove(e) {
    if (!mxDrag) return;
    const t = document.elementFromPoint(e.clientX, e.clientY);
    const cell = t && t.closest('.mx-cell');
    if (!cell) return;
    const cur = { row: +cell.dataset.row, col: +cell.dataset.col };
    if (
      !mxDrag.moved &&
      cur.row === mxDrag.anchor.row &&
      cur.col === mxDrag.anchor.col
    ) {
      return;
    }
    mxDrag.moved = true;
    mxDrag.cur = cur;
    updateMatrixSelection(cellBoundsFrom(mxDrag.anchor, cur));
  }

  function onMatrixDragUp() {
    document.removeEventListener('pointermove', onMatrixDragMove);
    if (!mxDrag) return;
    const d = mxDrag;
    mxDrag = null;
    if (d.moved) return; // 保留选区，底部浮出操作条
    // 单击：批量模式 → 选单格；否则新建草稿
    if (state.batchMode) {
      updateMatrixSelection({ r1: d.anchor.row, r2: d.anchor.row, c1: d.anchor.col, c2: d.anchor.col });
      return;
    }
    const rooms = filteredRooms();
    const room = rooms[d.anchor.row];
    const period = PERIODS[d.anchor.col % 5];
    const day = Math.floor(d.anchor.col / 5) + 1;
    if (!room) return;
    BLOCKS.push({
      id: nextId(),
      day,
      start: period.start,
      end: period.end,
      room: room.code,
      course: 'draft',
      override: false,
      draft: true,
    });
    recomputeConflicts();
    render();
    toast(`已为 ${room.code} ${weekdayName(day)} ${fmtTime(period.start)} 新建课程（草稿）`);
  }

  function openBatchApply(anchorEl) {
    const el = openPopover(
      `<div class="popover-head">批量应用课程<button type="button" class="icon-btn" data-pop="close"><svg class="ico"><use href="#i-x" /></svg></button></div>
       <div class="popover-body">
         <div class="field"><label>课程</label><select data-pop="course">${courseOptionsHtml('c4')}</select></div>
         <div class="popover-meta">将写入当前框选的时段，已占用的格子会自动跳过。</div>
       </div>
       <div class="popover-foot"><button type="button" class="btn btn-ghost" data-pop="close">取消</button>
       <button type="button" class="btn btn-primary" data-pop="apply">应用</button></div>`,
      { anchorEl, x: 0, y: 0 },
    );
    el.addEventListener('click', (e) => {
      if (e.target.closest('[data-pop="close"]')) return closePopover();
      if (e.target.closest('[data-pop="apply"]')) {
        const courseId = el.querySelector('[data-pop="course"]').value;
        applyBatch(courseId);
        closePopover();
      }
    });
  }

  function applyBatch(courseId) {
    const sel = state.matrixSel;
    if (!sel) return;
    const rooms = filteredRooms();
    const co = courseOf(courseId);
    let added = 0;
    let skipped = 0;
    for (let r = sel.r1; r <= sel.r2; r++) {
      const room = rooms[r];
      if (!room) continue;
      for (let c = sel.c1; c <= sel.c2; c++) {
        const day = Math.floor(c / 5) + 1;
        const period = PERIODS[c % 5];
        const occupied = BLOCKS.some(
          (b) =>
            b.room === room.code &&
            b.day === day &&
            overlap(b.start, b.end, period.start, period.end),
        );
        if (occupied) {
          skipped++;
          continue;
        }
        BLOCKS.push({
          id: nextId(),
          day,
          start: period.start,
          end: period.end,
          room: room.code,
          course: courseId,
          override: false,
          draft: false,
        });
        added++;
      }
    }
    state.matrixSel = null;
    recomputeConflicts();
    render();
    toast(
      `已将「${co.name}」应用到 ${added} 个时段${
        skipped ? `（跳过 ${skipped} 个已占用时段）` : ''
      }`,
      added ? '' : 'warn',
    );
  }

  /* ------------------------------ 渲染总控 ------------------------------ */
  function render() {
    recomputeConflicts();
    renderToolbar();
    renderWeek();
    renderTimeline();
    renderMatrix();
    renderLegend();
    renderDrawer();
  }

  /* ------------------------------ 视图切换 ------------------------------ */
  let hashLock = 0;

  function setHash(h) {
    const target = `#${h}`;
    if (location.hash === target) return;
    hashLock = 1;
    location.hash = target;
  }

  function applyView(view, opts) {
    state.view = view;
    $$('#view-seg .seg-item').forEach((b) =>
      b.classList.toggle('is-active', b.dataset.viewBtn === view),
    );
    ['week', 'timeline', 'matrix'].forEach((v) => {
      $(`#view-${v}`).classList.toggle('is-active', v === view);
    });
    $('#legend-overlay').style.display = view === 'timeline' ? '' : 'none';
    if (!opts || !opts.fromHash) setHash(view);
    closePopover();
    render();
  }

  function applyHash() {
    const raw = (location.hash || '').replace(/^#/, '');
    const [view, id] = raw.split('/');
    const v = ['week', 'timeline', 'matrix'].includes(view) ? view : 'week';
    state.drawerOpen = false;
    state.selectedId = null;
    applyView(v, { fromHash: true });
    if (id) openDrawer(id);
  }

  /* ------------------------------ 事件绑定 ------------------------------ */
  function bindEvents() {
    // 视图切换
    $('#view-seg').addEventListener('click', (e) => {
      const btn = e.target.closest('[data-view-btn]');
      if (!btn) return;
      applyView(btn.dataset.viewBtn);
    });

    // 周 / 日 导航
    $$('[data-week]').forEach((btn) =>
      btn.addEventListener('click', () => {
        const delta = +btn.dataset.week;
        if (delta === 0) state.weekOffset = 0;
        else state.weekOffset += delta;
        render();
      }),
    );
    $$('[data-day]').forEach((btn) =>
      btn.addEventListener('click', () => {
        const delta = +btn.dataset.day;
        let day = state.day + delta;
        if (day < 1) {
          day = 7;
          state.weekOffset -= 1;
        } else if (day > 7) {
          day = 1;
          state.weekOffset += 1;
        }
        state.day = day;
        render();
      }),
    );
    $('#btn-today').addEventListener('click', () => {
      state.weekOffset = 0;
      state.day = 1;
      render();
      toast('已定位到 2026-11-02 周一（演示日期）');
    });

    // 筛选
    $('#sel-area').addEventListener('change', (e) => {
      state.area = e.target.value;
      state.matrixSel = null;
      if (state.room !== 'all' && state.area !== 'all' && areaOfRoom(state.room) !== state.area) {
        state.room = 'all';
      }
      render();
    });
    const roomSel = $('#sel-room');
    ROOMS.forEach((r) => {
      const opt = document.createElement('option');
      opt.value = r.code;
      opt.textContent = `${r.code} · ${r.name}`;
      roomSel.appendChild(opt);
    });
    roomSel.addEventListener('change', (e) => {
      state.room = e.target.value;
      render();
    });
    $('#input-search').addEventListener('input', (e) => {
      state.search = e.target.value;
      state.matrixSel = null;
      render();
    });

    // 冲突聚焦
    $('#btn-conflict').addEventListener('click', () => {
      if (!CONFLICTS.pairs.length) {
        toast('当前筛选范围内没有冲突');
        return;
      }
      state.conflictFocus = !state.conflictFocus;
      render();
      toast(state.conflictFocus ? '已聚焦冲突课块，其它课块弱化显示' : '已退出冲突聚焦');
    });

    // 批量模式
    $('#input-batch').addEventListener('change', (e) => {
      state.batchMode = e.target.checked;
      state.matrixSel = null;
      applyMatrixSelection();
      toast(state.batchMode ? '批量模式：单击或拖拽选择时段' : '已退出批量模式');
    });

    // 新建 / 刷新
    $('#btn-new').addEventListener('click', () => openNewModal());
    $('#btn-refresh').addEventListener('click', () => {
      render();
      toast('已刷新排课数据（演示数据）');
    });

    // 侧边栏导航（演示）
    $$('.nav-item').forEach((item) => {
      item.addEventListener('click', (e) => {
        e.preventDefault();
        if (item.dataset.page === '课表') return;
        toast(`演示原型：当前仅实现「课表」页面（${item.dataset.page} 未实现）`);
      });
    });
    $('.user-collapse').addEventListener('click', () =>
      toast('演示原型：侧边栏收起未实现'),
    );
    $('.topbar-link').addEventListener('click', (e) => {
      e.preventDefault();
      toast('演示原型：API 文档为外部链接，离线原型未接入');
    });
    $('.bell').addEventListener('click', () => toast('演示原型：暂无通知'));
    $('.topbar-search input').addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        toast('演示原型：全局搜索未接入，请使用课表工具条内的搜索');
      }
    });

    // 课块点击（A / C 视图）→ 打开抽屉
    $('#stage').addEventListener('click', (e) => {
      const blockEl = e.target.closest('[data-block]');
      if (!blockEl) return;
      if (state.view === 'timeline') return; // 时间轴在 pointer 流程里处理
      openDrawer(blockEl.dataset.block);
    });

    // 时间轴：拖拽 / 空白新建
    $('#stage').addEventListener('pointerdown', (e) => {
      if (state.view !== 'timeline') return;
      if (e.button !== 0) return;
      if (e.target.closest('.drawer') || e.target.closest('.popover')) return;
      const blockEl = e.target.closest('.tl-block');
      if (blockEl) {
        startTimelineDrag(e, blockEl);
        return;
      }
      const track = e.target.closest('.tl-track');
      if (track) {
        const rect = track.getBoundingClientRect();
        const ppm = rect.width / DAY_SPAN;
        let start = snap(DAY_START + (e.clientX - rect.left) / ppm, 10);
        start = clamp(start, DAY_START, DAY_END - 100);
        openCreatePopover(
          track.dataset.room,
          start,
          start + 100,
          e.clientX - $('#stage').getBoundingClientRect().left + 14,
          e.clientY - $('#stage').getBoundingClientRect().top + 14,
        );
      }
    });

    // 矩阵：框选 / 新建草稿
    $('#stage').addEventListener('pointerdown', (e) => {
      if (state.view !== 'matrix') return;
      if (e.button !== 0) return;
      if (e.target.closest('.mx-block') || e.target.closest('.batch-bar') || e.target.closest('.popover'))
        return;
      const cell = e.target.closest('.mx-cell');
      if (!cell) return;
      startMatrixDrag(e, cell);
    });

    // 矩阵底部操作条 / 抽屉 / 弹层 / 模态
    document.addEventListener('click', (e) => {
      const batch = e.target.closest('[data-batch]');
      if (batch) {
        if (batch.dataset.batch === 'clear') {
          state.matrixSel = null;
          applyMatrixSelection();
        } else if (batch.dataset.batch === 'apply') {
          openBatchApply(batch);
        }
        return;
      }
      const dw = e.target.closest('[data-dw]');
      if (dw) {
        handleDrawerAction(dw.dataset.dw, dw);
        return;
      }
      const selOther = e.target.closest('[data-select-block]');
      if (selOther) {
        openDrawer(selOther.dataset.selectBlock);
        return;
      }
      const popClose = e.target.closest('[data-pop="close"]');
      if (popClose) closePopover();
    });

    // 复制到… 菜单
    document.addEventListener('click', (e) => {
      const dayBtn = e.target.closest('[data-copy-day]');
      if (!dayBtn) return;
      copyBlockToDay(+dayBtn.dataset.copyDay);
    });

    // 点击空白关闭弹层
    document.addEventListener(
      'pointerdown',
      (e) => {
        const pop = $('.popover');
        if (!pop) return;
        if (pop.contains(e.target)) return;
        if (e.target.closest('[data-dw="copy"]') || e.target.closest('[data-batch="apply"]')) return;
        closePopover();
      },
      true,
    );

    // 键盘
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        if ($('.popover')) return closePopover();
        if (!$('#modal-layer').hidden) return closeModal();
        if (state.drawerOpen) return closeDrawer();
        if (state.matrixSel) {
          state.matrixSel = null;
          applyMatrixSelection();
        }
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        $('#input-search').focus();
      }
    });

    let resizeTimer = null;
    window.addEventListener('resize', () => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => {
        if (state.view === 'matrix') renderMatrix();
      }, 150);
    });

    window.addEventListener('hashchange', () => {
      if (hashLock) {
        hashLock = 0;
        return;
      }
      applyHash();
    });
  }

  function handleDrawerAction(action, btn) {
    const b = BLOCKS.find((x) => x.id === state.selectedId);
    if (!b) return;
    const co = courseOf(b.course);
    if (action === 'close') return closeDrawer();
    if (action === 'edit') return toast('演示原型：编辑表单未接入，可先删除后新建');
    if (action === 'delete') {
      BLOCKS = BLOCKS.filter((x) => x.id !== b.id);
      closeDrawer();
      render();
      toast(`已删除「${co.name}」${weekdayName(b.day)} ${fmtRange(b.start, b.end)}（演示）`);
      return;
    }
    if (action === 'copy') {
      openPopover(
        `<div class="popover-head">复制到…<button type="button" class="icon-btn" data-pop="close"><svg class="ico"><use href="#i-x" /></svg></button></div>
         <div class="popover-body"><div class="popover-meta">${co.name} · ${b.room} · ${fmtRange(
          b.start,
          b.end,
        )}</div>
         <div class="day-menu">${DAYS.map(
           (d) => `<button type="button" data-copy-day="${d.i}">${d.name}</button>`,
         ).join('')}</div></div>`,
        { anchorEl: btn, x: 0, y: 0 },
      );
    }
  }

  function copyBlockToDay(day) {
    const b = BLOCKS.find((x) => x.id === state.selectedId);
    if (!b) return;
    const co = courseOf(b.course);
    const hit = BLOCKS.find(
      (x) =>
        x.id !== b.id &&
        x.room === b.room &&
        x.day === day &&
        overlap(x.start, x.end, b.start, b.end),
    );
    BLOCKS.push({
      id: nextId(),
      day,
      start: b.start,
      end: b.end,
      room: b.room,
      course: b.course,
      override: true,
      draft: false,
    });
    closePopover();
    closeDrawer();
    recomputeConflicts();
    render();
    toast(
      hit
        ? `已复制到 ${weekdayName(day)}（该时段与「${courseOf(hit.course).name}」冲突，请检查）`
        : `已复制「${co.name}」到 ${weekdayName(day)} ${fmtRange(b.start, b.end)}`,
      hit ? 'warn' : '',
    );
  }

  /* ------------------------------ 时钟 / 启动 ------------------------------ */
  function tickClock() {
    try {
      const t = new Intl.DateTimeFormat('en-GB', {
        timeZone: 'Asia/Shanghai',
        hour: '2-digit',
        minute: '2-digit',
        hour12: false,
      }).format(new Date());
      $('#sb-clock').textContent = t;
    } catch (err) {
      $('#sb-clock').textContent = '--:--';
    }
  }

  function boot() {
    bindEvents();
    tickClock();
    setInterval(tickClock, 30 * 1000);
    applyHash();
    // 曝光给自动化脚本 / 控制台调试：窗口可直接调用
    window.__proto = {
      state,
      blocks: () => BLOCKS,
      setView: (v) => applyView(v),
      openBlock: (id) => openDrawer(id),
      setDay: (d) => {
        state.day = d;
        render();
      },
      setWeek: (o) => {
        state.weekOffset = o;
        render();
      },
      toast,
    };
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
  } else {
    boot();
  }
})();
