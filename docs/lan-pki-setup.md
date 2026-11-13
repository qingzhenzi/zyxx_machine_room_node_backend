# 跨机 mTLS 配置教程（LAN IP SAN + device-pki 绑 0.0.0.0）

> 适用：控制节点与 agent 不在同一台机器（比赛现场 L1 + L2/L3 三台机）。
> 目标：**不用 SSH 反向隧道**，agent 直接通过局域网 enroll + mTLS 上线。
> 示例 IP：控制节点 `192.168.8.141`；两台学生机 `192.168.8.142/143`；网段 `192.168.8.0/24`。
> 仅用于开发/比赛演示，**不要用于生产**。

---

## 0. 先搞清楚三个地址（最容易错的地方）

| 地址 | 谁用 | 配在哪 |
|---|---|---|
| 注册链接里的地址 | agent enroll | qe-api 的 `PUBLIC_ENROLL_ENDPOINT`（默认 `https://localhost:4440`） |
| 证书签发给谁（SAN） | agent 校验服务端证书 | `gen_test_ca` 生成的 enroll/mtls 证书 |
| 注册后 mTLS 连哪 | agent 后续所有业务 | device-pki 的 `PUBLIC_MTLS_ENDPOINT`（随 enroll 响应下发给 agent 并存进凭证） |

三者必须一致（都写 LAN IP，或都写同一个域名）。**任何一处还是 localhost，跨机就会失败。**

---

## 1. 确认网络

```bash
# 控制节点
ip -4 addr show            # 找到 192.168.8.141 所在网卡
ping -c 1 192.168.8.142
# 学生机
ping -c 1 192.168.8.141
```

- 三台机同网段、同交换机；时间用 chrony 同步（证书校验依赖时间）。
- 控制节点建议固定 IP（不要 DHCP 漂移）。

---

## 2. 重新生成带 LAN SAN 的证书

`gen_test_ca.rs` 已支持额外 SAN（环境变量 `DEVICE_PKI_EXTRA_SANS`，逗号分隔 IP/域名；也支持命令行追加参数）。

```bash
cd room_node
DEVICE_PKI_EXTRA_SANS="192.168.8.141,kerno.local" bash scripts/dev-pki-certs.sh

# 校验（必须能看到 IP 和域名）
openssl x509 -in deploy/device-pki/certs/enroll.pem -noout -subject -ext subjectAltName
openssl x509 -in deploy/device-pki/certs/mtls.pem   -noout -subject -ext subjectAltName
```

预期输出包含：

```
X509v3 Subject Alternative Name:
    DNS:localhost, IP Address:127.0.0.1, IP Address:192.168.8.141, DNS:kerno.local
```

> ⚠️ **重新生成会换一张新 CA**：所有已注册的 agent 旧证书都会失效，必须重新 enroll。
> 如果不想动已注册设备，用文末「附录 A：保留原 CA，只补签服务端证书」。

---

## 3. 启动 device-pki：绑 0.0.0.0 + 公布 LAN 地址

### 方式 A（推荐，compose profile）

在 `room_node/.env` 里设置（compose 会读取）：

```bash
PUBLIC_ENROLL_ENDPOINT=https://192.168.8.141:4440
PUBLIC_MTLS_ENDPOINT=https://192.168.8.141:4441
DEVICE_PKI_ADMIN_TOKEN=<强随机值>
DEVICE_PKI_INTERNAL_TOKEN=<与 qe-api 的 DEVICE_PKI_INTERNAL_TOKEN 一致>
S3_ENDPOINT_DOCKER=http://rustfs:9000
S3_BUCKET=qestudio
S3_ACCESS_KEY=...
S3_SECRET_KEY=...
STORAGE_SIGNING_KEY_BASE64=<与 qe-api 一致>
```

然后：

```bash
make pki-dev        # 构建并启动 device-pki（compose 已固定 ENROLL_ADDR/MTLS_ADDR=0.0.0.0）
docker compose --profile pki ps
```

### 方式 B（本机二进制）

> **device-pki 不读 `.env`**，所有变量必须 `export`。

```bash
cd room_node
export DATABASE_URL=postgres://qestudio:qestudio@127.0.0.1:5432/qestudio
export ENROLL_ADDR=0.0.0.0:4440
export MTLS_ADDR=0.0.0.0:4441
export PUBLIC_ENROLL_ENDPOINT=https://192.168.8.141:4440
export PUBLIC_MTLS_ENDPOINT=https://192.168.8.141:4441
export ENROLL_TLS_CERT="$PWD/deploy/device-pki/certs/enroll.pem"
export ENROLL_TLS_KEY="$PWD/deploy/device-pki/certs/enroll-key.pem"
export MTLS_TLS_CERT="$PWD/deploy/device-pki/certs/mtls.pem"
export MTLS_TLS_KEY="$PWD/deploy/device-pki/certs/mtls-key.pem"
export CA_KEY="$PWD/deploy/device-pki/certs/ca-key.pem"
export CA_CERT="$PWD/deploy/device-pki/certs/ca.pem"
export ADMIN_TOKEN=<强随机值>
export INTERNAL_ADDR=127.0.0.1:4442
export INTERNAL_TOKEN=<与 qe-api 的 DEVICE_PKI_INTERNAL_TOKEN 一致>
export S3_ENDPOINT=http://127.0.0.1:9000
export S3_BUCKET=qestudio
export S3_ACCESS_KEY=...
export S3_SECRET_KEY=...
export STORAGE_SIGNING_KEY_BASE64=<与 qe-api 一致>
./target/debug/qe-device-pki
```

启动日志里应看到 `ENROLL_ADDR=0.0.0.0:4440` / `MTLS_ADDR=0.0.0.0:4441`（或 `ss -ltnp | grep 444` 确认监听在 `0.0.0.0`）。

---

## 4. 改 qe-api 的 `.env` 并重启

```bash
# room_node/.env
PUBLIC_ENROLL_ENDPOINT=https://192.168.8.141:4440
BIND_ADDR=0.0.0.0:8080
DEVICE_PKI_INTERNAL_URL=http://127.0.0.1:4442
DEVICE_PKI_INTERNAL_TOKEN=<与 device-pki INTERNAL_TOKEN 一致>
# S3_* / STORAGE_SIGNING_KEY_BASE64 必须与 device-pki 完全一致
```

重启 `qe-api`。之后控制台签发的注册链接应以 `https://192.168.8.141:4440/enroll?...` 开头。

---

## 5. 放行端口

```bash
# ufw
sudo ufw allow 4440/tcp
sudo ufw allow 4441/tcp
sudo ufw allow 8080/tcp
# 若要用学生机浏览器访问控制台
sudo ufw allow 5174/tcp

# firewalld
sudo firewall-cmd --add-port=4440-4441/tcp --permanent
sudo firewall-cmd --add-port=8080/tcp --permanent
sudo firewall-cmd --reload
```

> Vite dev 默认只监听 `127.0.0.1:5174`。如果评委/学生机要用浏览器开控制台，
> 用 `pnpm dev -- --host 0.0.0.0`（或改 `frontend/vite.config.ts` 的 `server.host`）。

---

## 6. 学生机 enroll

1. 控制台「房间/设备」页签发注册链接，**确认链接 host 是 192.168.8.141**；
2. 在学生机执行：

```bash
sudo qe-agent cli enroll "https://192.168.8.141:4440/enroll?token=...&slot=..." --trust
qe-agent cli status
```

`cli status` 里的 `mtls_endpoint` 必须是 `https://192.168.8.141:4441`（不是 localhost）。

3. `/etc/qe-agent.env`（参考 `room_agent/apps/qe-agent/deploy/qe-agent.env.example`）：

```bash
DEVICE_PKI_MTLS_URL=https://192.168.8.141:4441
DEVICE_SLOT=<slot>
DEVICE_DATA_DIR=/var/lib/qe-agent
```

4. 启动：

```bash
sudo systemctl restart qe-agent
# 或前台
sudo qe-agent serve --role both --ipc-socket /run/qe-agent/agent.sock
```

5. 控制台 `/machines` 应显示在线；device-pki 日志出现：

```
机器快照已上报（mTLS） serial=... slot=... tier="fast" health=ok
```

---

## 7. 验证清单

```bash
# ① 证书 SAN
openssl x509 -in deploy/device-pki/certs/mtls.pem -noout -ext subjectAltName

# ② TCP 监听
ss -ltnp | grep -E '4440|4441'

# ③ 服务端证书（带 SNI）
openssl s_client -connect 192.168.8.141:4440 -servername 192.168.8.141 </dev/null 2>/dev/null \
  | openssl x509 -noout -subject -ext subjectAltName

# ④ 从学生机可达
curl -k -m 5 https://192.168.8.141:4440/crl -o /dev/null -w '%{http_code}\n'

# ⑤ agent 端
qe-agent cli status | grep -iE 'mtls|enrolled|slot|serial'

# ⑥ 控制台/日志
#    /machines 在线 + device-pki 日志出现“机器快照已上报（mTLS）”
```

---

## 8. 常见错误对照

| 现象 | 原因 | 处理 |
|---|---|---|
| `certificate not valid for name` / `InvalidCertificate` | 证书 SAN 里没有这个 IP/域名 | 第 2 步重签，然后重新 enroll |
| `Connection refused` | 仍绑 `127.0.0.1` | `ENROLL_ADDR=0.0.0.0:4440`、`MTLS_ADDR=0.0.0.0:4441` |
| 注册链接还是 `localhost` | qe-api 的 `PUBLIC_ENROLL_ENDPOINT` 没改 / 没重启 | 改 `.env` 并重启 qe-api |
| enroll 成功但 mTLS 连不上 | device-pki 的 `PUBLIC_MTLS_ENDPOINT` 没改 / 没重启 | 改后重启；`cli status` 看 `mtls_endpoint` |
| `502` / `503 storage_unavailable` | device-pki 缺 `S3_*` 或签名密钥与 qe-api 不一致 | 两边 export 同一套；重启 |
| 所有旧设备突然离线 | 重签换了 CA | 重新 enroll（或用附录 A 保留原 CA） |
| noVNC 卡“正在连接” | `INTERNAL_TOKEN` 不一致 / nginx 无 WS Upgrade | 两边 token 一致；演示走 Vite dev |
| 证书时间错误 | 机器时间不同步 | `chronyd` / `timedatectl` 校时 |

---

## 附录 A：保留原 CA，只补签服务端证书（不重装已注册设备）

前提：`deploy/device-pki/certs/ca.pem` 与 `ca-key.pem` 是当前设备信任的 CA（不要删）。

```bash
cd deploy/device-pki/certs

# 1) 生成新密钥 + CSR（ECDSA P-256，与 rcgen 默认一致）
openssl ecparam -name prime256v1 -genkey -noout -out enroll-key.pem.new
openssl req -new -key enroll-key.pem.new -subj "/CN=192.168.8.141" -out enroll.csr

# 2) SAN 扩展配置
cat > san.cnf <<'EOF'
subjectAltName=DNS:localhost,IP:127.0.0.1,IP:192.168.8.141,DNS:kerno.local
basicConstraints=CA:FALSE
keyUsage=digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
EOF

# 3) 用现有 CA 签（3650 天）
openssl x509 -req -in enroll.csr -CA ca.pem -CAkey ca-key.pem \
  -CAcreateserial -days 3650 -sha256 -extfile san.cnf -out enroll.pem.new

# 4) 对 mtls 重复 1-3（CN 可写 mtls 或同 IP）
openssl ecparam -name prime256v1 -genkey -noout -out mtls-key.pem.new
openssl req -new -key mtls-key.pem.new -subj "/CN=192.168.8.141" -out mtls.csr
openssl x509 -req -in mtls.csr -CA ca.pem -CAkey ca-key.pem \
  -CAcreateserial -days 3650 -sha256 -extfile san.cnf -out mtls.pem.new

# 5) 替换并重启 device-pki（compose: make pki-dev；二进制：重启进程）
mv enroll.pem.new enroll.pem && mv enroll-key.pem.new enroll-key.pem
mv mtls.pem.new mtls.pem       && mv mtls-key.pem.new mtls-key.pem
```

> 设备证书由同一张 CA 签发，所以已注册设备**继续有效**；只需要重启 device-pki 让新服务端证书生效。

---

## 附录 B：回滚到单机

```bash
cd room_node
bash scripts/dev-pki-certs.sh          # 不带 EXTRA_SANS，恢复 localhost/127.0.0.1
# 同时把 qe-api/.env 与 device-pki 的 PUBLIC_* 改回 https://localhost:4440 / 4441
# 重新 enroll 本机 agent（旧证书随 CA 更换失效）
```

---

## 安全注意

- 这套是**开发自签证书**（CA 私钥在仓库目录、10 年有效期），只用于比赛演示；
- 演示机不要连公网；`ADMIN_TOKEN` / `INTERNAL_TOKEN` / S3 密钥不要用默认值；
- 赛后把 `deploy/device-pki/certs/` 从交付物里排除（已在 `.gitignore`）。
