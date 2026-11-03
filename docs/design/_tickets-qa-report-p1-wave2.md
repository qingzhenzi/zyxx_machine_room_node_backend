# 工单 P1 Wave 2 独立 QA 验证报告（根因 / 数据范围 / 附件 / 报修入口）

> 验证人：qa-verifier（独立 QA，未参与实现）｜时间：2026-11-02｜对象：`room_node_backend` 工单 P1 Wave 2
> 依据：`docs/design/ticket-p1-contract.md` §11（Wave 2）+ Wave 1/P0 契约
> 仓库路径已迁移：**`/home/qingzhenzi/Projects/QEStudio/room_node/room_node_backend`**（前端在 `room_node_backend/frontend`；旧路径已空）
> 方式：只读代码审查 + 契约验收命令 + 真实 PG/Redis 集成测试 + 运行栈（API `127.0.0.1:8080`、Vite `127.0.0.1:5175`、RustFS S3 `127.0.0.1:9000`）的 API/DB/浏览器实测。
> 只写本报告，未改源码；临时数据已清理（§9）。

---

## 1. 结论（verdict）

> **复验更新（2026-11-02 19:56，第二轮）**：P1-1（附件上传 CORS）与 P2（多段权限码）已修复并通过定向复验（后端代理上传 + Playwright 浏览器上传成功；RBAC 多段权限 201/PUT 200/非法 400）。**最终 verdict：PASS**（遗留 P3 不阻塞；详见 §12）。
> 以下 §1–§11 为首轮（修复前）原始结论，保留作历史基线；问题状态以 §12 为准。

**PASS-with-issues（首轮结论，已被 §12 复验取代）**（含 **1 个 release-blocker 候选**：前端附件上传被对象存储 CORS 阻断，见 P1-1）。后端 API、RBAC 后端判定、根因、数据范围、附件元数据全链路、报修预填、Wave1/P0 回归均实测通过；存在 1 个高优先级交付/配置缺口 + 1 个 P2 权限编码兼容缺陷 + 若干 P3。

| # | 严重度 | 问题 | 位置 |
|---|--------|------|------|
| 1 | **P1（blocker 候选）** | **前端附件上传在交付环境被 RustFS CORS 阻断**：浏览器直传预签名 PUT 被 `Access-Control-Allow-Origin` 缺失拦截，页面 toast「上传附件失败」；后端 API 全链路（presign→PUT→register→list→delete）用 curl 实测通过。根因是对象存储 bucket 未配置 CORS（`.env` 的 `CORS_ALLOWED_ORIGINS` 仅作用于后端 API） | `frontend/src/features/tickets/TicketAttachments.vue:91-105`（浏览器 `fetch` 直传）；部署配置缺 RustFS CORS（`compose.yaml`/`deploy/`/`.env:34`） |
| 2 | P2 | **RBAC 权限编码校验拒绝双冒号权限**：`POST /admin/roles` 与 `PUT /admin/roles/{id}/permissions` 对 `ticket:rule:manage`（Wave1）、`ticket:scope:all`、`ticket:scope:manage`（Wave2）均返回 400「权限编码格式应为 <资源>:<动作>」。导致自定义角色无法被授予这些新权限（仅 admin/迁移种子角色拥有），前端角色页编辑含这些权限的角色保存会失败；`ticket:scope:all` 无法通过产品 API 授予（QA 验证时用 SQL 直插绕过） | `crates/domain/src/validation.rs:147-157`（`action.contains(':')` 直接拒绝）；受影响权限来自 `migrations/0021_ticket_rbac.sql`、`migrations/0024_ticket_scope.sql` |
| 3 | P3 | `PUT /admin/ticket-scopes/{user_id}` 对**没有 `ticket:read` 的目标用户**会先写入 `ticket_scopes`、写审计，然后因重新查询用户列表（只含 ticket:read 用户）而返回 **404**（写入后 404，语义不一致、调用方误判为未生效） | `crates/api/src/api/admin/ticket_scopes.rs:38-69`；`crates/db/src/ticket_scopes.rs:99-140` |
| 4 | P3 | 不存在的区域在 `PUT scopes` 返回 **404**「包含不存在的区域」（其它规则/参数校验多为 400），状态码口径不一致 | `crates/db/src/ticket_scopes.rs`（`replace` 返回 `DbError::not_found`） |
| 5 | P3 | `POST /tickets/{id}/attachments`（register）响应 `uploaded_by_name` 恒为 `null`（SQL `RETURNING NULL::text`），而 list 返回名称；且 register 不校验对象是否真实存在/sha 是否匹配（仅校验元数据），可登记未上传成功的 key | `crates/db/src/ticket_attachments.rs`（insert RETURNING）；`crates/api/src/api/admin/ticket_attachments.rs:125-160` |
| 6 | P3 | 附件 DELETE 只删元数据，**对象存储对象保留**（无清理/GC）；删除后桶内会残留孤儿对象 | `crates/api/src/api/admin/ticket_attachments.rs:207-260`、`crates/db/src/ticket_attachments.rs`（delete 仅 DELETE 行） |
| 7 | P3 | 附件前端下载用 `window.open(download_url, '_blank')`（非强制下载）：实测新标签打开 S3 URL 并显示文本内容；对 inline 类型是“预览”而非“下载”，且 S3 响应未设置 `Content-Disposition: attachment` | `frontend/src/features/tickets/TicketAttachments.vue:130-136` |
| 8 | P3 | 前端「可见范围」页的区域下拉依赖 `area:read`；仅有 `ticket:scope:manage` 而无 area/room read 的角色会拿不到区域列表（代码注释称回退 UUID，UI 实际可用性未端到端验证）。与 Wave1 规则页 scope 依赖类似，属权限独立授权的 UI 耦合 | `frontend/src/views/TicketScopesView.vue:84-96` |
| 9 | P3 | 报修入口的标题预填使用房间/设备/机器名称（如“设备报修：room-101”“机器报修：cachyos-qingzhenzi”），可创建；但 `serial/device_slot` 与标题写在同一 query 中，用户可在弹窗中改标题——可接受，仅提示 | `frontend/src/views/{Room,Device,Machine}DetailView.vue` |

---

## 2. 环境与验证范围

- 后端 `127.0.0.1:8080`：migration `0023_ticket_root_cause` / `0024_ticket_scope` / `0025_ticket_attachment` 已应用（`_sqlx_migrations` 23/24/25 success=t）。
- 前端 Vite `127.0.0.1:5175`；对象存储 RustFS `S3_ENDPOINT=http://127.0.0.1:9000`、bucket `qestudio`、path-style。
- 仓库迁移后验证：后端可编译/运行、前端可构建/运行、OpenAPI/生成类型/api client/路由均可用（§7）。
- 未重启/未 kill 服务、未改源码、无外网请求；临时数据已清理（§9）。

---

## 3. 验收命令结果（逐条实测）

| # | 命令 | 结果 |
|---|------|------|
| 1 | `cargo fmt --check -p qe-api -p qe-domain -p qe-db` | ✅ FMT_EXIT=0 |
| 2 | `cargo check -p qe-api` | ✅ Finished“dev” |
| 3 | `cargo test -p qe-domain` | ✅ **99 passed; 0 failed**（新增 root_cause/attachment 校验/文件名 sanitize 等） |
| 4 | `cargo test -p qe-db` | ✅ **20 passed; 0 failed** |
| 5 | `cargo test -p qe-api --lib` | ✅ **66 passed; 0 failed** |
| 6 | `cargo test -p qe-api --test tickets -- --ignored --test-threads=1`（TEST_DATABASE_URL=qestudio_test / TEST_REDIS_URL=6379） | ✅ **6 passed; 0 failed**（15.97s；新增 `wave2_attachments_register_list_and_delete`、`wave2_data_scopes_limit_reads_and_writes`） |
| 7 | `vue-tsc --build` | ✅ exit 0 |
| 8 | `vitest run` | ✅ 8 files / **101 tests passed**（Wave1/P0 无回归） |
| 9 | `vite build` | ✅ 成功（4.40s） |
| 10 | `oxfmt --check src` | ✅ All matched files use the correct format（192 files） |
| 11 | `oxlint src` | ✅ 0 warnings / 0 errors（191 files） |
| 12 | `E2E_BASE_URL=http://127.0.0.1:5175 E2E_HEADLESS=1 playwright test e2e/tickets.spec.ts --project=chromium` | ✅ **6 passed**（20.0s；含报修预填、可见范围页；e2e 未覆盖附件上传，故未发现 P1-1） |
| 13 | 生成物一致性 | ✅ live `/api-docs/openapi.json` 与 `frontend/docs/openapi.json` canonical **完全一致**；新增 5 条路径（ticket-scopes×2、attachments×3），`root_cause`/`TicketScopeUser`/`TicketAttachment` 在 schema 与 `schema.ts` 均存在 |

---

## 4. 根因分类（契约 §11 A）

- 6 个值逐一实测：创建→claim→resolve(root_cause=hardware/network/software/config/usage/other) 全部 **200** 且 DB `root_cause` 精确落库（6/6）。
- 非法值 `bogus` → **400**（“非法的根因分类…”）。
- `resolved→closed` 可在关闭时**补充**根因（无根因 resolve 后 close 带 `network` → DB=`network`，200）。
- 详情 API 返回 `ticket.root_cause`；前端详情 InfoList 显示「根因分类：硬件故障」；OpenAPI/生成类型含 6 值枚举（含 null）。
- 说明：`transition` 对 `in_progress/cancelled/open` 目标的 `root_cause` 会被忽略（DB 仅 resolve/close 分支写），符合契约。

## 5. 数据范围（契约 §11 B）

权限与配置：
- `GET /admin/ticket-scopes`：admin/`ticket:scope:manage`/`ticket:manage` 可读；普通 `ticket:read` 用户 **403**；`PUT` 同权限；不存在用户 → 404；不存在区域 → 404（见问题 #4）。
- `ticket:scope:all` 用户：可见全部（实测 list total=17、可 get 区域 B 工单）。
- 未配置任何 scope 的用户：**不受限**（向后兼容，实测可 get 区域 B 工单）。
- 审计：`ticket_scope.update` 落库（`resource_type="ticket_scope"`、`resource_id=user_id`、detail 含 area_ids）。

过滤效果（配置用户范围=[区域 A] 后）：
- 列表：total=1，等于 DB 中区域 A 工单真值（1），不含区域 B；
- 详情：区域 A 工单 **200**，区域 B 工单 **404**（按不存在处理，不泄露）；
- 写操作：claim/transition 区域 B → **403**（“无权操作该区域的工单”）；区域 A claim → **200**；
- 统计：`stats.totals.created`=1，与区域 A 真值一致；
- 导出：CSV 含 A 不含 B；
- 替换语义：`[A]→[B]` 后仅见 B；`[]` 清空后恢复不受限（含 B）；区域去重/排序由 repo 完成。
- 前端：`/tickets/scopes` 页面 admin / manage-only（仅 `ticket:read+ticket:manage`）均可进入（permissionAny），表格 14 行、编辑按钮可用、保存调用 PUT 成功（API 实测）；路由在 `:id` 之前。

## 6. 附件（契约 §11 C）—— API 全链路 PASS，浏览器上传 FAIL（P1-1）

后端 API（curl，规则：20MiB、sha256、类型白名单、object_key 前缀）：
- presign 合法 → 200（object_key=`tickets/{ticket}/{uuid}/{filename}`、upload_url、expires_at）；
- 预签名 PUT 到 RustFS → **200**；register → **201**；重复同一 `object_key` → **409**（唯一冲突映射 Conflict）；
- list → 200，含 `download_url`（预签名 GET）与 `uploaded_by_name`；下载 URL 取回内容与上传一致；
- 校验（全部 400）：size=0、size=20MiB+1（20971521）、sha256 非 64hex、content-type 非白名单、filename 为空；工单不存在 → 404；路径穿越文件名 `../../etc/passwd` 被 sanitize 为 `passwd`（object_key 不越界）；
- 权限：`ticket:read` 用户 list 200、presign **403**；范围外工单 presign **403**；非上传者删除 **403**；`ticket:manage` 删除他人附件 **204**；上传者删除 **204**；
- 审计：`ticket_attachment.create` / `ticket_attachment.delete` 落库；
- storage 503：代码审查（`storage_service()` 未配置 → 503；`storage_error()` 把存储错误映射 503），因环境已配置存储未做 live 503 注入。

**浏览器端（P1-1）**：详情页「上传附件」→ presign 成功后浏览器 `fetch PUT http://127.0.0.1:9000/...` 被 CORS 拦截（console：`Access to fetch at ... has been blocked` + `ERR_FAILED`），页面显示「上传附件失败」；实测 `OPTIONS` 预检 200 但**无 Access-Control-Allow-Origin**。附件空态文案、附件列表、下载按钮均正常；下载按钮 `window.open` 实测新标签打开 S3 URL 并显示内容（可“预览”，非强制下载）。
→ 结论：**后端附件能力完整；前端上传在当前交付配置下不可用**（需 bucket CORS 或改为后端代理上传），e2e 未覆盖该路径。

## 7. 报修入口 / 仓库迁移/接线核对

- 房间详情「报修/建单」：e2e 用例 `opens create-ticket modal prefilled from room 报修 entry` **通过**；
- 设备详情：点击「报修 / 建单」→ 跳 `/tickets` 并打开新建弹窗，标题预填「设备报修：room-101」、携带 serial/device_slot；
- 机器详情：同样入口，标题预填「机器报修：cachyos-qingzhenzi」；
- 仓库迁移后 Lead 补的接线：`crates/api/src/api/admin/mod.rs` 注册 5 条新路由（附件静态段在 `/tickets/{id}` 前、`ticket-scopes` 独立段）；`frontend/src/api/index.ts` 新增 listTicketScopes/setTicketScopes/listTicketAttachments/presignTicketAttachment/registerTicketAttachment/deleteTicketAttachment；`frontend/src/api/schema.ts` 含 `root_cause`、`TicketScopeUser`、`TicketAttachment`、`UpdateTicketScopesRequest`；`routes.ts`/`AppSidebar.vue` 增加 `/tickets/scopes`（`permissionAny:[scope:manage, manage]`）与侧边栏项；README/CONVENTIONS 已更新。**核对结果：接线正确、可编译可运行。**

## 8. Wave1 / P0 回归

- `cargo test -p qe-domain` 99、`-p qe-db` 20、`-p qe-api --lib` 66、tickets 集成 6（含既有 P0/Wave1 用例）全绿；
- frontend vitest 101（含 Wave1 规则/门控/通知相关既有套件）、e2e 6/6（P0→Wave1→Wave2 页面链路）；
- OpenAPI 与生成类型一致、无 stray 文件/依赖新增（`git status` 除本次功能/文档/仓库重组外无临时脚本）。

---

## 9. 未验证项

1. **RustFS CORS 修复后的浏览器上传**：当前环境未配置 CORS，无法验证修复效果（P1-1 已定性）。
2. **storage 未配置 503 live**：环境已配置 RustFS，仅代码审查。
3. **前端可见范围保存 UI**：页面加载/API PUT 已验证；弹窗内区域多选→保存的完整 UI 未端到端点击验证。
4. **无 area:read 的 scope:manage 角色在 UI 选择区域**：代码审查发现下拉依赖 area:read（P3-8），未构造该角色端到端。
5. **附件 UI 删除**：因上传被 CORS 阻断，无法在浏览器端到端完成“上传→删除”；后端删除/权限已 API 实测。
6. **Firefox/WebKit e2e、全仓 cargo test**：按任务范围未跑。
7. **S3 孤儿对象清理**：删除附件后对象残留（P3-6），未清理桶内对象。

## 10. 数据清理

- 临时数据已清理：13 张 QA 工单、15 个临时用户、15 个临时角色、ticket_scopes 行、ticket_attachments 行全部删除；审计按 append-only 保留（actor_id 已置空）。
- dev 库恢复：**5 张原有演示工单 + 6 条种子规则**，无 QA 用户/角色/附件/范围行。
- 说明：Q/A 上传到 RustFS 的少量对象（`tickets/{id}/.../note.txt` 等）因附件删除仅删元数据而残留（见 P3-6），未使用 S3 凭据做对象级清理。

## 11. 复现命令（关键）

```bash
cd /home/qingzhenzi/Projects/QEStudio/room_node/room_node_backend
cargo fmt --check -p qe-api -p qe-domain -p qe-db
cargo check -p qe-api && cargo test -p qe-domain && cargo test -p qe-db && cargo test -p qe-api --lib
TEST_DATABASE_URL=postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test \
TEST_REDIS_URL=redis://127.0.0.1:6379 \
  cargo test -p qe-api --test tickets -- --ignored --test-threads=1
cd frontend
./node_modules/.bin/vue-tsc --build && ./node_modules/.bin/vitest run && ./node_modules/.bin/vite build
./node_modules/.bin/oxfmt --check src && ./node_modules/.bin/oxlint src
E2E_BASE_URL=http://127.0.0.1:5175 E2E_HEADLESS=1 \
  ./node_modules/.bin/playwright test e2e/tickets.spec.ts --project=chromium

# P1-1 复现：登录 → /tickets/{id} 详情 → 「上传附件」→ 浏览器 console 出现
#   Access to fetch at 'http://127.0.0.1:9000/qestudio/...' from origin 'http://127.0.0.1:5175' has been blocked by CORS policy
# P2 复现：POST /api/v1/admin/roles {"permission_codes":["ticket:read","ticket:scope:all"]} → 400
```

---

## 12. 复验结果（修复后定向复验，2026-11-02 19:45–19:56）

Lead 已修复本轮两个问题；本轮做**定向复验**（未改源码、未重启服务），结论：**PASS（两个问题均关闭；残留 P3 不阻塞）**。

### 12.1 修复项复验

| 原问题 | 修复验证 | 结果 |
|---|---|---|
| P1-1 附件上传 CORS blocker | 新增后端代理上传 `POST /admin/tickets/{id}/attachments/upload?filename=&content_type=`（原始字节、服务端 sha256、`put_bytes`、登记；`crates/api/src/api/admin/ticket_attachments.rs:135-230`，路由 `admin/mod.rs:249`）；前端改用 `api.uploadTicketAttachment`（同源，`frontend/src/api/index.ts:763`，`TicketAttachments.vue:84`）。**curl 原始字节上传 → 201**，`sha256` 与本地计算一致，list 返回 `download_url` 且下载内容一致，上传者 delete 204；空 body → 400；非法 content_type → 400；**预签名入口仍可用**（200）。**Playwright 浏览器 file chooser 上传成功**：toast「附件已上传」、详情附件列表出现 `r2-body.txt`（`download_url` 非空），**无 CORS console error**、无 500（此前 3 条 500 为并发压测期瞬时登录 500，非上传路径）。`.env`/`.env.example` 已设 `MAX_BODY_BYTES=26214400`（25MiB） | ✅ 关闭（CORS blocker 解除；代理上传成为默认路径） |
| P2 RBAC 双冒号权限码 | `normalize_permission_code` 改为允许 **2–4 段**且拒绝空段/非法字符（`crates/domain/src/validation.rs:147-175`）。实测：`POST /admin/roles` 授予 `ticket:read + ticket:rule:manage + ticket:scope:all` → **201**（返回 permissions 含多段码）；`PUT /admin/roles/{id}/permissions` 授予 `ticket:scope:manage/scope:all` → **200**；5 段 `a:b:c:d:e` → 400、`ticket::read` → 400、`ticket:read:` → 400（提示“<资源>:<动作> 或 <资源>:<子资源>:<动作>”）；授权后用户 `/auth/me` 实际获得 `ticket:rule:manage/scope:all/scope:manage` 并可访问 `/admin/ticket-scopes`（200）。domain 单测 99 通过（含新增多段权限测试）、集成回归 6 通过 | ✅ 关闭 |

### 12.2 命令复跑（全部通过）

| 命令 | 结果 |
|---|---|
| `cargo fmt --check -p qe-api -p qe-domain -p qe-db` | ✅ FMT_EXIT=0 |
| `cargo check -p qe-api` | ✅ Finished（31.42s 冷编译） |
| `cargo test -p qe-domain` | ✅ 99 passed |
| `cargo test -p qe-db` | ✅ 20 passed |
| `cargo test -p qe-api --lib` | ✅ 66 passed |
| `cargo test -p qe-api --test tickets -- --ignored --test-threads=1` | ✅ 6 passed（16.33s） |
| `vue-tsc --build` | ✅ exit 0 |
| `vitest run` | ✅ 8 files / 101 passed |
| `vite build` | ✅ 成功 |
| `oxfmt --check src` | ✅ 全绿 |
| `oxlint src` | ✅ 0 warnings / 0 errors |
| Playwright e2e（5175） | ⚠️ 与 cargo 编译/多浏览器并发同时跑时 3 failed / 2 flaky / 1 passed（页面 30s 加载超时 + 偶发登录 500，属并发资源竞争）；**单独复跑（无并发负载）：6 passed + 1 skipped（13.6s）** ✅ |

### 12.3 残留/未关闭项（均不阻塞，与首轮 P3 一致）

- **P3** `PUT /admin/ticket-scopes/{user_id}` 对没有 `ticket:read` 的用户仍是“先写入再 404”（`ticket_scopes.rs`）。
- **P3** 附件 `register` 返回 `uploaded_by_name=null`、不校验对象存在/sha（代理上传路径已由服务端计算 sha，但 presign+register API 客户端路径仍可登记未上传 key）。
- **P3** 附件 DELETE 只删元数据，S3 对象残留（无 GC/清理）。
- **P3** 前端下载仍用 `window.open`（S3 链接预览，非强制下载；代理上传后下载 URL 仍为预签名 GET）。
- **P3** 可见范围页区域选择依赖 `area:read`；无 area:read 的 scope:manage 角色 UI 可用性未端到端验证。
- **未完全验证**：`MAX_BODY_BYTES=26214400` 的 live 进程未重启（当前进程仍加载旧配置），故 20MiB+1 请求返回 **413**（body limit 而非元数据 400）；配置/代码正确，重启后应走 400（建议发布时重启并复测）；storage 503 live 未注入（代码审查）；Firefox/WebKit 未跑。
- 说明：本轮 e2e 并发阶段出现一次 `/auth/login` **500**（latency 1558ms，服务端日志确认），无并发单独复跑未复现；疑为 QA 并发压测（cargo 集成测试+e2e+Playwright 同时占用 DB 连接池）导致，建议关注连接池/DB 池上限（非工单逻辑改动引入）。

### 12.4 数据清理

本轮临时数据已清理：R2 QA 工单 4 张、临时用户/角色、附件元数据、`ticket_scopes` 行；dev 库恢复 **5 张演示工单 + 6 条种子规则**，无 QA 用户/角色/附件/范围行。RustFS 中本轮上传的少量对象（代理上传文件）因删除仅删元数据而残留（P3）。

```
