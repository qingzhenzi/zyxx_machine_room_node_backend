# Kubernetes 主动摘流

开源 nginx 只有被动健康检查（`max_fails` / `fail_timeout`），Docker healthcheck
也不会自动把副本从 DNS/upstream 移除。生产环境如果有多副本，建议用 K8s
Service/Endpoints 或云 LB 做主动摘流。

`deploy/k8s/deployment.yaml` 提供了最小清单：

- `readinessProbe: /readyz`：数据库/Redis 不可用时不再接新流量
- `livenessProbe: /healthz`：只判断进程存活，避免数据库抖动导致重启风暴
- `preStop: sleep 10`：等待 Endpoints 摘流和 nginx 连接排空
- `terminationGracePeriodSeconds: 35`
- `RollingUpdate maxUnavailable: 0`
- `PodDisruptionBudget minAvailable: 1`

使用：

```bash
kubectl apply -f deploy/k8s/deployment.yaml
```

发布前把镜像和 `qestudio-api` Secret 换成真实值。Secret 至少包含
`DATABASE_URL`、`REDIS_URL`、`JWT_SECRET`、`PUBLIC_BASE_URL`、`SMTP_*`。

如果前面仍是 nginx：

- nginx `upstream` 只做被动检查，不能替代 K8s readiness
- 让 nginx 指向 K8s Service/Ingress，而不是直接解析 Pod IP
- 保留 `proxy_next_upstream http_502 http_503`，用于已建立连接的兜底

云 LB（ALB/NLB/SLB）可配置主动健康检查 `/readyz`，效果等同。
