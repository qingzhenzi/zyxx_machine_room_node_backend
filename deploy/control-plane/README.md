# 控制面 systemd（开发机）

`qe-api` 和 `qe-device-pki` 不能只靠前台进程；否则控制面退出后主机会一直
`Connection refused`，表现为机器离线/无法获取状态。

安装：

```bash
sudo cp qe-api.service qe-device-pki.service /etc/systemd/system/
sudo cp device-pki.env.example /etc/qe-control-device-pki.env
sudo nano /etc/qe-control-device-pki.env   # 调整路径/密钥
sudo systemctl daemon-reload
sudo systemctl enable --now qe-api qe-device-pki
ss -ltnp | grep -E ':(8080|4440|4441|4442)'
```

验证：

```bash
curl -s http://127.0.0.1:8080/healthz
# 管理端机器列表应显示 dev-b104 online
```
