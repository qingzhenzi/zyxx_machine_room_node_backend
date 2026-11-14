# 教室主机复现手册（dev-b104 / i5-12500 / Debian 13）

> 2026-11-14 实测。覆盖：1Panel 卸载与防火墙接管、VM 规格优化、NAT/DNS/公网、
> RDP 1920×1080 kiosk、教师端 RDP、SR-IOV PF/VF。所有命令均在
> `192.168.8.141:2222`（root/123456）执行，控制节点 `192.168.8.231`。
> 备份/回滚见文末。

## 0. 硬件/系统基线

```text
CPU: i5-12500 (6C/12T)
GPU: Intel UHD Graphics 770 [8086:4690]，i915，IOMMU group 0
OS: Debian 13 trixie，kernel 6.12.116+deb13-amd64
KVM/QEMU: libvirt 11.3 / QEMU 10.0.13
显示: KDE Plasma Wayland，HDMI-A-1 1920x1080
VM: win10-ltsc，q35，6 vCPU/8GB，VGA，SATA，e1000，VNC 5901
```

## 1. 卸载 1Panel，QEStudio 接管防火墙

```bash
# 备份
tar czf /root/qe-backup/1panel-$(date +%s).tgz \
  /opt/1panel /etc/1panel /usr/local/bin/1panel-* \
  /etc/systemd/system/1panel-*.service
iptables-save > /root/qe-backup/iptables-before.txt

# 停 1Panel
systemctl disable --now 1panel-core.service 1panel-agent.service

# 删 1Panel 防火墙链（不要动 LIBVIRT/DOCKER）
for chain in 1PANEL_BASIC_BEFORE 1PANEL_BASIC 1PANEL_BASIC_AFTER ONEPANEL_INPUT; do
  while iptables -C INPUT -j "$chain" 2>/dev/null; do iptables -D INPUT -j "$chain"; done
  iptables -F "$chain" 2>/dev/null || true
  iptables -X "$chain" 2>/dev/null || true
done
# IPv6 同理：ip6tables -C/-D/-F/-X

iptables -P INPUT ACCEPT
iptables -P FORWARD ACCEPT
rm -f /etc/systemd/system/1panel-core.service /etc/systemd/system/1panel-agent.service
rm -rf /opt/1panel /etc/1panel
rm -f /usr/local/bin/1panel-core /usr/local/bin/1panel-agent /usr/local/bin/1panel-kvm
systemctl daemon-reload

# 安装 qe-firewall
install -m 0644 deploy/qe-firewall/qe-base.nft /etc/qe-firewall.nft
install -m 0755 deploy/qe-firewall/qe-firewall-apply /usr/local/sbin/qe-firewall-apply
install -m 0755 deploy/qe-firewall/qe-firewall-watchdog /usr/local/sbin/qe-firewall-watchdog
install -m 0644 deploy/qe-firewall/qe-firewall.service /etc/systemd/system/
systemctl daemon-reload && systemctl enable --now qe-firewall.service
nft list table inet qe_base
```

`qe_base` 只放行：loopback、established/related、ICMP、tcp 22/2222/13389、`qevm-*` 到宿主。
VM 策略由 `inet qe_vm` 管理。回滚材料 `/root/qe-backup/`。

## 2. VM 规格优化（只重启 guest）

```bash
virsh shutdown win10-ltsc
cp --reflink=auto /var/lib/libvirt/images/win10-ltsc.qcow2 \
   /var/lib/libvirt/images/win10-ltsc.qcow2.bak-$(date +%s)
virsh dumpxml win10-ltsc > /tmp/win10-before.xml

# XML 关键改动
# memory/currentMemory 4194304 -> 8388608
# vcpu 4 -> 6
# cpu mode=host-passthrough + topology sockets=1 dies=1 cores=6 threads=1
# features 加 <hyperv> relaxed/vapic/spinlocks/vpindex/runtime/synic/stimer/reset/frequencies
# clock offset='localtime' + <timer name='hypervclock' present='yes'/>
# graphics 加 sharePolicy='force-shared'
virsh define /tmp/win10-optimized.xml
virsh start win10-ltsc
```

Windows 内关闭透明/动画、高性能电源；VGA 只支持 1024×768，清晰 1080p 走 RDP。

## 3. NAT + 公网 + DNS forwarder

startup profile（`PUT /api/v1/admin/rooms/{room_id}/startup`）里 VM 的 network：

```json
{
  "mode": "nat",
  "allow_services": [{"cidr": "192.168.8.231/32", "proto": "tcp", "ports": [8080, 4441, 9000]}],
  "allow_internet": true,
  "allow_same_room": false
}
```

- `qe-vm-net` 生成 libvirt NAT + nft allowlist；`allow_internet` 可运行时切换：
  ```bash
  qe-agent cli vm net internet --room <room_id> --on/--off --apply
  ```
- DNS：agent 从 `/etc/resolv.conf` 自动取 IPv4 forwarder 写入 `<dns><forwarder>`；
  宿主防火墙必须放行隔离桥 DHCP/DNS/NTP（`qe-vm-net ensure` 已幂等补 iptables 例外）。

## 4. RDP 1920×1080 主显示

### Windows 侧（离线 hive）

```bash
# 关 VM 后 qemu-nbd 挂载 Windows 系统盘
modprobe nbd max_part=8
qemu-nbd -c /dev/nbd0 /var/lib/libvirt/images/win10-ltsc.qcow2
partprobe /dev/nbd0
ntfsfix -d /dev/nbd0p2
mount -t ntfs-3g -o rw,remove_hiberfile /dev/nbd0p2 /mnt/win

# python3-hivex 修改 SYSTEM：
# ControlSet001\Control\Terminal Server\fDenyTSConnections = 0
# ControlSet001\Services\TermService\Start = 2
# FirewallPolicy\{Standard,Domain,Public}Profile\EnableFirewall = 0
python3 - <<'PY'
import hivex, struct
h=hivex.Hivex('/mnt/win/Windows/System32/config/SYSTEM', write=True)
root=h.root()
def child(n,name): return h.node_get_child(n,name)
def dw(n,k,v): h.node_set_value(n,{"key":k,"t":4,"value":struct.pack("<I",v)})
c=child(root,"ControlSet001")
dw(child(child(c,"Control"),"Terminal Server"),"fDenyTSConnections",0)
dw(child(child(c,"Services"),"TermService"),"Start",2)
fw=child(child(child(child(c,"Services"),"SharedAccess"),"Parameters"),"FirewallPolicy")
for p in ("StandardProfile","DomainProfile","PublicProfile"):
    dw(child(fw,p),"EnableFirewall",0)
h.commit(None)
PY
umount /mnt/win; qemu-nbd -d /dev/nbd0
virsh start win10-ltsc
nc -z 10.77.148.174 3389   # RDP_OPEN
```

### 主机 kiosk

```bash
apt-get install -y freerdp3-x11 socat
install -m 0755 deploy/rdp-kiosk/qe-rdp-kiosk.sh /usr/local/bin/qe-rdp-kiosk.sh
install -m 0644 deploy/rdp-kiosk/qe-rdp-kiosk.desktop \
  /home/roomuser/.config/autostart/qe-rdp-kiosk.desktop
chown roomuser:roomuser /home/roomuser/.config/autostart/qe-rdp-kiosk.desktop
echo 'QE_BOOT_DISPLAY_VM_AFTER_STARTUP=0' >> /etc/qe-agent.env
systemctl restart qe-agent
```

当前进程：

```text
xfreerdp3 /v:10.77.148.174 /u:qe /p:*** /size:1920x1080 /f /cert:ignore
```

### 教师端

```bash
install -m 0644 deploy/rdp-kiosk/qe-rdp-forward.service /etc/systemd/system/
systemctl daemon-reload && systemctl enable --now qe-rdp-forward.service
```

教师笔记本：RDP `192.168.8.141:13389`，账号 `qe / QeStudio2026`。
VM IP 变化时改 service 的 `TCP:` 目标。

## 5. SR-IOV（UHD 770，i915-sriov-dkms）

```bash
# 1) DKMS（6.12 用 2025.12.10.2）
apt-get install -y linux-headers-$(uname -r) dkms build-essential
dpkg -i i915-sriov-dkms_2025.12.10.2_amd64.deb
dkms status

# 2) GRUB
GRUB_CMDLINE_LINUX_DEFAULT="quiet intel_iommu=on iommu=pt \
i915.enable_guc=3 i915.max_vfs=7 i915.xelp_enable_ccs=1 module_blacklist=xe"
update-grub && update-initramfs -u -k all
reboot

# 3) 验证 PF
dmesg | grep -E "SR-IOV PF mode|VUs|VFs|GUC: submission enabled"
cat /sys/bus/pci/devices/0000:00:02.0/sriov_totalvfs   # 7

# 4) 创建 VF + vfio + VM hostdev（自动化）
install -m 0755 deploy/sriov/qe-sriov-vf /usr/local/sbin/qe-sriov-vf
install -m 0644 deploy/sriov/qe-sriov-vf.service /etc/systemd/system/
systemctl daemon-reload && systemctl enable --now qe-sriov-vf.service

virsh dumpxml win10-ltsc > /tmp/win10-vf.xml
# </devices> 前加：
# <hostdev mode='subsystem' type='pci' managed='yes'>
#   <source><address domain='0x0000' bus='0x00' slot='0x02' function='0x1'/></source>
# </hostdev>
virsh define /tmp/win10-vf.xml
virsh start win10-ltsc
```

当前证据：

```text
i915 0000:00:02.0: Running in SR-IOV PF mode
i915 0000:00:02.0: GUC: submission enabled
i915 0000:00:02.0: 7 VFs could be associated with this PF
0000:00:02.1 ... driver=vfio-pci
```

Windows guest 还需要 Intel VF 显示驱动 + GOP/主显示配置，QSV/Sunshine 才可用；
见 `room_agent/docs/sriov-host-setup.md` 与 `tmp/gpu-sunshine-runbook.md`。

## 6. 检查清单

```bash
nft list table inet qe_base
nft list table inet qe_vm
cat /sys/bus/pci/devices/0000:00:02.0/sriov_numvfs
basename $(readlink -f /sys/bus/pci/devices/0000:00:02.1/driver)
virsh dumpxml win10-ltsc | grep -E "memory|vcpu|hostdev|hyperv|sharePolicy"
nc -z 192.168.8.141 13389
systemctl is-active qe-firewall qe-rdp-forward qe-sriov-vf qe-agent
```

## 7. 回滚

```bash
# RDP
rm -f /home/roomuser/.config/autostart/qe-rdp-kiosk.desktop
sed -i '/QE_BOOT_DISPLAY_VM_AFTER_STARTUP/d' /etc/qe-agent.env
systemctl restart qe-agent

# SR-IOV
systemctl disable --now qe-sriov-vf.service
echo 0 > /sys/bus/pci/devices/0000:00:02.0/sriov_numvfs
cp /root/qe-sriov-backup/grub /etc/default/grub
update-grub && update-initramfs -u -k all
# 从 VM XML 移除 hostdev，否则重启 VM 会找不到 VF
dkms remove i915-sriov-dkms/2025.12.10.2 --all
reboot

# 1Panel
tar xzf /root/qe-backup/1panel-*.tgz -C /
systemctl daemon-reload && systemctl enable --now 1panel-core 1panel-agent
```

## 8. RDP 内自动化（本次实测）

窗口化 RDP 可以用 X11 截图/键鼠驱动，便于在 Windows 内操作：

```bash
# 启动窗口化 RDP（non-fullscreen，便于 import 截图）
XAUTH=$(ls /run/user/1000/xauth_* | head -1)
runuser -u roomuser -- env DISPLAY=:0 XAUTHORITY=$XAUTH \
  XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 GDK_BACKEND=x11 \
  xfreerdp3 /v:10.77.148.174 /u:qe /p:'QeStudio2026' /size:1600x900 /cert:ignore +clipboard &

ID=$(runuser -u roomuser -- env DISPLAY=:0 XAUTHORITY=$XAUTH xdotool search --name FreeRDP | head -1)
runuser -u roomuser -- env DISPLAY=:0 XAUTHORITY=$XAUTH import -window "$ID" /tmp/rdp.png
runuser -u roomuser -- env DISPLAY=:0 XAUTHORITY=$XAUTH xdotool mousemove --window "$ID" 164 811 click 1
```

已确认 Windows 内 VF 出现在设备管理器：

```text
PCI\VEN_8086&DEV_4690&SUBSYS_89B4103C
描述: Microsoft 基本显示适配器
问题代码 31 (CM_PROB_FAILED_ADD)
```

说明 VF 已成功分配给 Windows；下一步是安装 Intel 桌面显卡驱动（LTSC 2021 21H2 可能需要 30.x/31.x 版本），
Windows Update 自动搜索当前返回“最佳驱动已安装”，需要手动 Intel 驱动包。

## 9. 当前未闭环

- Windows Intel VF 驱动未安装（QSV/3D 尚未验证）；
- Sunshine/Moonlight 未安装；
- RDP kiosk 当前直连 `10.77.148.174`，IP 变需同步。

## 10. Windows VF 驱动安装（实测 2026-11-14）

Windows 设备管理器已看到 VF：

```text
PCI\VEN_8086&DEV_4690&SUBSYS_89B4103C
```

但初始为 Microsoft 基本显示适配器，问题代码 31。Windows Update 自动搜索返回“最佳驱动已安装”，
需要手动装 Intel DCH 驱动。

### 10.1 驱动包

Microsoft Update Catalog（Windows 10 Vibranium/21H2 匹配）：

- UpdateID：`402bba47-2671-43c4-9088-4421dcc1ce4e`
- 版本：**32.0.101.7088**
- CAB 直链：
  `https://catalog.s.download.windowsupdate.com/d/msdownload/update/driver/drvs/2026/09/258793aa-b29f-4640-bee8-dedfec94faba_fc5b16dc2adfc2c14dedeba820fbaffb5027a44b.cab`
- 大小约 550MB。

### 10.2 送进 guest

主机上启动临时 HTTP（仅演示 LAN/隔离网）：

```bash
cd /tmp; python3 -m http.server 18080 --bind 0.0.0.0 --directory /tmp
# 允许 guest 访问宿主 18080
nft insert rule inet qe_vm input iifname qevm-0945adec ip daddr 10.77.148.1 tcp dport 18080 accept
```

Windows（管理员 cmd）：

```bat
curl -L -o C:\Users\qe\driver.cab http://10.77.148.1:18080/intel-driver.cab
mkdir C:\Users\qe\driver
expand -F:* C:\Users\qe\driver.cab C:\Users\qe\driver
pnputil /add-driver C:\Users\qe\driver\*.inf /install /subdirs
```

安装输出：

```text
驱动程序包已安装在设备上: PCI\VEN_8086&DEV_4690&SUBSYS_89B4103C...
已添加驱动程序包数: 6
```

### 10.3 生效验证

重启 guest 后：

```bat
wmic path win32_VideoController get Name,DriverVersion
```

实测输出：

```text
10.0.19041.868  Microsoft Remote Display Adapter
10.0.19041.868  Microsoft 基本显示适配器
32.0.101.7088    Intel(R) UHD Graphics 770
```

`pnputil /enum-devices /problem` 中 Intel VF 不再出现（仅剩 QEMU ACPI 和 virtio 未装的两项）。

### 10.4 下一步

- QSV 验证：任务管理器 GPU / `ffmpeg -encoders | findstr qsv` + 实际编码；
- Sunshine 安装并选 QuickSync；
- 若 Sunshine 捕获黑屏，加 `ramfb=on` 或虚拟显示驱动；
- 当前 RDP kiosk 仍可用，作为回退。
