# 真机验收清单：多机 P2P / WOL 真实唤醒 / noVNC 真机键鼠

> 本文件是**未执行**的真机验收脚本与判定标准。当前开发机只有单台物理机 + 本机 libvirt VM，
> 因此 `tmp/sim/`（SIMULATION）、本机 VM 的 noVNC 键鼠自动化都**不能**替代以下真机验收。
> 执行人需准备真实教室网络后，按本清单留证并回填结果。

## 0. 环境前提

- 2 台以上物理机（建议 A=控制节点 + agent，B/C=学生机 agent；至少 2 台可对比 P2P/HTTP）。
- 同一台交换机（或跨交换机 VLAN 互通），能抓端口计数器；固定 IP/主机名，agent 已 enroll。
- 镜像仓库可写（rustfs/S3），已发布过一个 ≥1GiB 的镜像（建议真实 OS 底包，用于 P2P/传输对比）。
- WOL 目标机：主板/网卡支持 WOL，BIOS 打开 Wake on LAN，操作系统关机为 S5（或 S3），
  与发送端同一广播域（或交换机已配置 directed broadcast）。
- 控制节点与 agent 时间同步（chrony），日志保留 `tmp/logs/real-acceptance-<date>/`。

## 1. 真实多机 P2P（跨交换机）

### 1.1 对照设计

| 组 | 配置 | 观测 |
|---|---|---|
| HTTP 基线 | 所有 agent `QE_P2P_ENABLED=0`（默认） | 控制节点/镜像中心出口字节 ≈ N×镜像大小；墙钟 |
| P2P 单 seeder | seeder 机 `QE_P2P_ENABLED=1`，先 pull+share | 镜像中心出口 ≈ 1×镜像；seeder 出口 ≈ N×；墙钟 |
| P2P 双 seeder | 两台 seeder 已 share，N 台同时 pull | 镜像中心出口 ≈ 1~2×镜像；**必须记录墙钟是否改善** |

### 1.2 步骤（每档重复 3 次，取中位数）

```bash
# seeder 机：先拉一次并保持在线（自动 share）
QE_P2P_ENABLED=1 qe-agent serve --role both ...
# 其它机器同时拉同一 digest（通过控制台“分发镜像”/API 触发，或 agent cli image pull）
# 抓证据：
#  - agent 日志：p2p hit/miss、peer 数、回退 HTTP、校验通过
#  - 交换机端口计数器（in/out octets）或控制节点 iftop/nload
#  - 墙钟：time 或任务 created_at→completed_at
#  - 逐片 sha256 校验全通过（镜像中心 verify 或 agent 日志）
```

### 1.3 判定

- [ ] 每台机器镜像校验 PASS（N/N）
- [ ] P2P 组镜像中心出口字节显著低于 HTTP 组（目标 ≤1.2×镜像大小/图片数）
- [ ] 墙钟：**以实测为准，不预设“更快”**；若更慢需给出原因（单机 CPU/网络/peer 选择策略）
- [ ] 失败回退：断掉 seeder 后仍能 HTTP 拉取成功
- [ ] 跨交换机场景记录 VLAN/MTU/组播或 mDNS 行为（qep2p 依赖 mDNS 发现时）

## 2. WOL 真实唤醒

### 2.1 前提

- 目标机网卡 MAC 已在平台登记（agent 上报物理网卡 MAC，见 `room_agent` telemetry `sections.agent.host.macs`）。
- 目标机处于 S5，交换机/路由器允许广播。
- 发送端与目标机同 L2 广播域（或配置 IP 定向广播）。

### 2.2 步骤

```bash
# 1) 目标机正常关机（S5），确认关机而非休眠；记录时间
# 2) 控制台/API 触发唤醒（device:wake 权限）
curl -X POST http://<control>:8080/api/v1/admin/machines/<serial>/wake \
  -H "Authorization: Bearer <admin-token>"
# 3) 发送端抓包（与目标机同广播域的另一台机器）
sudo tcpdump -i <iface> -vv -X 'udp port 9 or udp port 7'
# 4) 目标机 60s 内启动；agent 上线（device_session online / 机器总览显示在线）
```

### 2.3 判定

- [ ] 抓包 payload = 6×0xFF + 目标 MAC×16（102B），UDP 目标端口 9（或配置端口）
- [ ] 目标机真实上电启动（不是模拟 MAC/抓包）
- [ ] agent 自动上线并恢复心跳/遥测
- [ ] 失败时记录：BIOS WOL 设置、网卡指示灯、交换机广播/风暴抑制、VLAN

## 3. noVNC 真机 + 真人键鼠

- 真实教室电脑/服务器（非本机测试 VM）安装 agent，注册到控制节点。
- 控制台机器详情 →「控制台」→ 浏览器打开 canvas；用真人键鼠完成：
  - [ ] 能看到真实桌面/BIOS 画面
  - [ ] 移动鼠标光标（Raw 编码下画面有响应）
  - [ ] 点击/双击/右键菜单可用
  - [ ] 键盘输入（字母/回车/方向键）可见
  - [ ] 关闭弹窗后 agent 隧道释放（日志 reason、无残留 TCP/WS）
  - [ ] 弱网（`tc netem 50ms/1% loss`）下仍可操作，无明显卡死
- 当前已在本机真实 libvirt VM（edudock-node01，1280×800）用 RFB 协议自动化验证
  session→WS→中继→agent→VNC 且输入后收到 FramebufferUpdate（`tmp/logs/novnc-interactive-live.log`）；
  **真人+真实教室机仍需按本节补验。**

## 4. 结果回填模板

```text
日期/执行人/环境（拓扑、交换机、带宽）：
P2P 结果（3 次中位数）：HTTP 墙钟/出口字节；P2P 单/双 seeder 墙钟/出口字节；校验通过率
WOL 结果：抓包文件、目标机启动时间、agent 上线时间、异常
noVNC 结果：截图/录屏、输入响应、弱网表现、异常
结论：可宣称（勾选实际支持项）/不可宣称项：
```

## 5. 2026-11-12 真机联动（基础链路，已执行）

### 5.1 被测机器

```text
地址: 192.168.8.141
系统: Debian GNU/Linux 13 (trixie)
桌面: SDDM + graphical.target；普通用户 roomuser；Intel UHD 770
规格: x86_64 / 15 GiB RAM / 452 GiB NVMe（已用 8.5G）
```

### 5.2 安装与注册

- 用 release bundle `qestudio-agent-0.1.0`（本地构建版）安装：
  `install.sh --yes --gui-user roomuser --allow-poweroff ...`
- 设备：`serial=06386505f8b49f248a27793b2c50a8b44fb1069e`，`slot=dev-b104`
- 房间：`dev-b104 / B104 创新工坊`，注册状态 `active`
- 因开发 PKI 证书 SAN 只有 localhost/127.0.0.1，真机通过 **SSH 反向隧道**
  `localhost:4440/4441 -> 控制节点 127.0.0.1:4440/4441` 完成注册；
  生产环境应给 PKI 证书加 LAN IP SAN，并绑定 `0.0.0.0`。

### 5.3 已验证结果

- [x] `qe-agent` systemd active (running)
- [x] 首次 full 遥测上报成功（HTTP 202，snapshot 入库）
- [x] 控制面 API 返回：`status=active`、`online=online`、`session_online=online`
- [x] 机器详情显示真实主机名 `DESKTOP-ROOM-AGENT`、Debian 13、CPU/内存/磁盘与运行时长
- [x] 控制台发起 `telemetry/report` 立即上报命令，API 返回 202
- [x] 真实机器出现在房间 B104；截图：`tmp/screenshots/real-machine-debian13.png`
- [ ] WOL 真实唤醒（未测，需关机/上电条件）
- [ ] P2P 多机（未测，只 1 台真机）
- [ ] noVNC 真人键鼠（未测）
- [ ] 客户端制作镜像/下发（未测）

### 5.4 已发现的问题

- `boot-display` 由 root systemd 的 agent 直接 spawn 时没有 `DISPLAY`/`XAUTHORITY`，
  GTK 初始化失败并 core dump（`Failed to initialize GTK`）。已在真机临时移除
  `QE_BOOT_DISPLAY_BIN`，避免崩溃重启；后续需要改为在图形会话内启动，或由
  agent 显式注入用户 DISPLAY/XAUTHORITY。
- 本次使用 SSH 隧道绕过证书 SAN 限制，**不能视为生产可达性验收**；
  生产需 LAN 监听 + 正确 SAN + 防火墙放行 4440/4441。
