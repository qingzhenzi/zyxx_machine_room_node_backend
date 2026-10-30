# Kubernetes 主动摘流

开源 nginx 只有被动健康检查（`max_fails` / `fail_timeout`），Docker healthcheck
也不会自动把副本从 DNS/upstream 移除。生产环境如果有多副本，建议用 K8s
Service/Endpoints 或云 LB 做主动摘流。

`deploy/k8s/deployment.yaml` 提供了最小清单：

- `startupProbe: /healthz`：给迁移/初始化留出时间，避免启动慢被 liveness 重启
- `readinessProbe: /readyz`：数据库/Redis 不可用时不再接新流量
- `livenessProbe: /healthz`：只判断进程存活，避免数据库抖动导致重启风暴
- `preStop: sleep 10`：等待 Endpoints 摘流和 nginx 连接排空
- `terminationGracePeriodSeconds: 35`
- `RollingUpdate maxUnavailable: 0`、`PodDisruptionBudget minAvailable: 1`
- 非 root、只读根文件系统、drop ALL capabilities、`/tmp` emptyDir
- `automountServiceAccountToken: false`

使用：

```bash
kubectl apply -f deploy/k8s/deployment.yaml
```

发布前把镜像换成 CI 产出的不可变 tag，并创建 `qestudio-api` Secret：

```bash
kubectl create secret generic qestudio-api \
  --from-literal=APP_ENV=production \
  --from-literal=DATABASE_URL='postgres://user:pass@postgres:5432/db' \
  --from-literal=REDIS_URL='redis://redis:6379' \
  --from-literal=JWT_SECRET="$(openssl rand -hex 32)" \
  --from-literal=BOOTSTRAP_ADMIN_USERNAME=admin \
  --from-literal=BOOTSTRAP_ADMIN_PASSWORD='<strong-password>' \
  --from-literal=PUBLIC_BASE_URL='https://app.example.com' \
  --from-literal=CORS_ALLOWED_ORIGINS='https://app.example.com' \
  --from-literal=EMAIL_DRIVER=smtp \
  --from-literal=SMTP_URL='smtp://user:pass@smtp.example.com:587' \
  --from-literal=SMTP_FROM='Acme <no-reply@example.com>'
```

`APP_ENV=production` 会拒绝示例 JWT/管理员密码和通配 CORS。生产还应补上
Ingress/TLS、NetworkPolicy、HPA、外部 Secret 管理；该清单是 starter，不是开箱即用的
production hardening 方案。

如果前面仍是 nginx：

- nginx `upstream` 只做被动检查，不能替代 K8s readiness
- 让 nginx 指向 K8s Service/Ingress，而不是直接解析 Pod IP
- 保留 `proxy_next_upstream http_502 http_503`，用于已建立连接的兜底

云 LB（ALB/NLB/SLB）可配置主动健康检查 `/readyz`，效果等同。
