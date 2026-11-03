# 工单 P1 Wave 3 巡检计划 QA 报告

> 时间：2026-11-03｜对象：`room_node_backend`（仓库已迁移至 `QEStudio/room_node/room_node_backend`，控制台在 `frontend/`）
> 依据：`docs/design/ticket-p1-contract.md` §13（Wave3）+ §1–12（Wave1/2）
> 说明：独立 QA（qa-verifier）本轮执行了部分用例后被中断，未产出报告；以下为 **Lead 在最终代码/服务状态上的完整复验**，命令与环境均真实执行，临时数据已清理。

## 1. 结论

**PASS-with-issues（无 release blocker）**。巡检计划后端/worker/API/前端/权限/回归全部通过；Wave3 期间发现并修复了两个真实缺陷：

1. **手动执行不推进 `next_run_at`**：计划 `next_run_at` 为 NULL 时，手动 run 后 worker 会在下一 tick 再次执行，导致每 tick 重复建单。修复：`finish_run` 同时 `next_run_at = now() + interval_hours`；集成测试改为显式把计划置到期再验证并发 claim。
2. **上传 body 上限未真正放大**：只改 `RequestBodyLimitLayer` 时 Axum `DefaultBodyLimit` 仍为 2MiB，20MiB 附件在到达校验前被断开。修复：路由层同时应用 `DefaultBodyLimit::max(MAX_BODY_BYTES)`；实测 21MiB → 400（业务校验），1MiB → 201。

## 2. 命令矩阵（全绿）

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --check -p qe-api -p qe-domain -p qe-db` | exit 0 |
| `cargo check -p qe-api` | ok |
| `cargo test -p qe-domain` | **100 passed** |
| `cargo test -p qe-db` | **20 passed** |
| `cargo test -p qe-api --lib` | **66 passed**（含 OpenAPI 断言） |
| `cargo test -p qe-api --test tickets -- --ignored --test-threads=1`（qestudio_test） | **7 passed**（P0 + W1 + W2 + W3，含并发 claim、手动 run 建单字段、禁用不执行、body 上限回归） |
| `vue-tsc --build` | 0 errors |
| `vitest run` | **112 passed / 9 files** |
| `vite build` / `oxfmt --check src` / `oxlint src` | ✅ / ✅ / 0 warnings |
| Playwright e2e（5175） | **7/7 passed**（P0 + 报修预填 + rules + scopes + inspections） |
| `dump_openapi` | 106 paths，合法 JSON |

## 3. 功能实测

- **迁移**：dev 库 `_sqlx_migrations` version=26 `ticket inspection` success；test 库同样成功。
- **权限**：`ticket:inspection:manage` 已播种并加入 `rbac::permissions::ALL`（ALL=36）；admin `/auth/me` 共 36 权限且含该权限；operator/ticket_operator 不授予（集成测试 403）；`ticket:manage` 兼容。
- **CRUD/校验**：名称 1–100、间隔 1–168、area/room 必须 scope_id 且存在；PATCH 部分更新/清空语义；DELETE 级联 runs；审计 `inspection_plan.create/update/delete/run`；以上由集成测试覆盖。
- **执行**：手动 `POST /inspection-plans/{id}/run` → status=issues、checked=1、issue=1、ticket_id 非空；工单 `category=machine_health/source=auto/source_type=inspection/source_id=run_id/dedupe=inspection:{plan}:{run}`、priority=normal（无 critical）；run 历史 `GET /inspection-runs?plan_id=&limit=` 过滤/上限正确。
- **并发/防重**：`claim_due_plans` 两并发只命中一次、claim 后不再到期、禁用计划不 claim、worker tick 执行到期计划且不重复建单（集成测试 + live worker 日志）。
- **修复回归**：手动 run 后 `next_run_at` 为未来时间（live：`每日机房巡检` next_run_at=true，未再每 tick 建单）；21MiB 上传 400、1MiB 上传 201。
- **前端**：`/tickets/inspections` 页面渲染计划表（名称/范围/间隔/启用/最近运行/下次运行/操作）、新建/编辑/立即运行/删除/运行历史面板、空/403 状态；导航「巡检计划」可见；e2e 通过。
- **回归**：Wave1（RBAC/rules/SSE/CSV）、Wave2（根因/范围/附件/报修入口）、P0 全部测试与 e2e 通过；`frontend/docs/openapi.json` 与后端 spec 一致（106 paths）。

## 4. 遗留（P3，不阻塞交付）

1. 巡检工单 `serial/device_slot` 为空（多机器巡检单，异常清单在 description/metadata.summary 与 run.summary）。
2. 运行历史面板默认折叠；计划列表“最近运行”只展示摘要，详细历史需展开（设计如此）。
3. Wave2 遗留：`PUT /ticket-scopes` 对无 `ticket:read` 用户先写后 404；附件 register 不校验对象存在/`uploaded_by_name` 可为 null；删除附件不清理 S3 对象（dev 桶可能残留少量对象）；下载 `window.open` 为预览。
4. 未验证环境项：真实 SMTP、CSV >5000 行、提醒双副本并发、Firefox/WebKit。
5. 数据：dev 库保留 5 张演示工单 + 1 个演示巡检计划/1 次运行/1 张巡检工单；RustFS 可能有 QA 残留对象（仅元数据删除）。

## 5. 复验入口

```bash
cd /home/qingzhenzi/Projects/QEStudio/room_node/room_node_backend
cargo fmt --check -p qe-api -p qe-domain -p qe-db && cargo check -p qe-api
cargo test -p qe-domain && cargo test -p qe-db && cargo test -p qe-api --lib
TEST_DATABASE_URL=postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test \
TEST_REDIS_URL=redis://127.0.0.1:6379 \
cargo test -p qe-api --test tickets -- --ignored --test-threads=1
cd frontend && ./node_modules/.bin/vue-tsc --build && ./node_modules/.bin/vitest run && ./node_modules/.bin/vite build
E2E_BASE_URL=http://127.0.0.1:5175 E2E_HEADLESS=1 ./node_modules/.bin/playwright test e2e/tickets.spec.ts --project=chromium
```


## 6. P3 修复复验（2026-11-03）

针对本报告 §4 与 Wave2 遗留 P3，已完成：

| 问题 | 修复 | 验证 |
| --- | --- | --- |
| 附件下载是 `window.open` 预览 | 新增后端代理下载 `GET .../attachments/{attachment_id}/download`，`Content-Disposition: attachment`；前端改为 blob 下载 | live 下载内容一致；e2e/单测通过 |
| 删除附件不清理 S3 对象 | `StorageService::delete_object` + 删除后 best-effort 清理 | live 删除后原预签名 URL → 404 |
| register 不校验对象存在 | `object_exists` HEAD 校验，不存在 400 | 代码审查 + 编译/测试 |
| scope 页依赖 area:read | 新增 `GET /admin/ticket-scopes/areas`（scope 管理权限） | live 200、13 个区域；新增集成测试 viewer 403 |
| 低权限首页 403 噪声 | overview/shell 请求按权限懒加载 | `ticket_operator` 登录首页 403 网络请求 0 |
| CSV>5000 行未验证 | 新增集成测试：插入 5001 行 → 导出 5000 行数据（5001 行含表头） | 集成测试通过 |
| 提醒双副本并发未验证 | 新增集成测试：两并发 `claim_reminder` 仅一次成功 | 集成测试通过 |
