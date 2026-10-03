# Axum Admin 开发路线图

> 基线：`7f4d5dbb`（v0.19.0 已推送）· 编制日期：2026-10-03
> 本文取代 `NEXT_VERSION_SCOPE.md` 作为**前瞻性**文档。后者是 v0.2.0 的历史执行记录，已封存。
> 本文所有"缺口"均为 2026-10-03 实测确认，不是推测。每条都标了证据位置。

---

## 0. 先说三件不是功能的事

**发布元数据缺口（阻塞项）。** v0.19.0 三个提交已推送，但版号仍是 `0.18.0`，
没有 v0.19.0 的 tag / CHANGELOG 条目 / Release。tag 只到 `v0.16.0`。
后果是 `git describe` 失效、CI 版本注入继续报 0.18.0，而 README 的功能清单已经写到 v0.19.0 时代——
**文档与版号已经对不上**。这比任何新功能都更该先修。

**悬空的文件上传组件。** `frontend/src/components/common/BaseUpload.vue` 是一个完成度很高的上传组件
（拖拽区 / 分片进度 / 重试取消 / 大小与类型限制），已从 `components/common/index.ts:11` 导出，
但**全仓无人使用**。后端 `axum` 只开了 `features=["macros"]`，**没开 multipart**；
`tower-http` 只开了 `cors, trace, set-header`，**没开 `fs`**；没有任何上传端点。
`views/demo/index.vue:60` 里还留着一句"BaseUpload 暂时不在这里演示"。
一个对外导出的组件指向一个不存在的服务端，是会误导后来人的悬空承诺——要么补齐后端，要么摘掉它。

**测试规模已经很可观。** 集成测试 227 个函数、133 条 ignored 集成用例、单测 81 条、前端 161 条。
新增能力的测试成本要算进工作量，不能当免费附加项。

---

## 1. v0.20.0 —— 账号自持 + 管理可应急

**主题**：让用户能自己维护账号，让管理员在出事时有手动手段。
两条线都是"闭环已有机制"，不是开新战场，因此适合放在一版里。

### A 线：用户自助（3 项）

| 编号 | 功能 | 为什么现在做 | 证据 |
|---|---|---|---|
| A1 | 个人资料端点 + 资料字段 | `/api/auth/*` 下只有 `password` 是 PUT，**没有 profile 端点**；`users` 表 8 列里没有 nickname/avatar/display_name；`profile/index.vue` 只有改密三个字段。用户对自己的账号**没有任何自助修改能力** | `router/mod.rs:147-159`、`migrations/001_create_users.sql`、`views/profile/index.vue` |
| A2 | 用户列表按角色 / 状态筛选 | `list_users` 只收 `page/page_size/keyword`，keyword 同时匹配 username+email。管理员找"某个角色的禁用账号"只能靠翻页 | `controller/user.rs` 的 utoipa 参数表 |
| A3 | 头像上传（配 A1） | 悬空组件已就绪，但需要开 multipart + 落盘策略，见下方风险 | 全仓 grep 零命中后端实现 |

### B 线：管理应急（3 项）

| 编号 | 功能 | 为什么现在做 | 证据 |
|---|---|---|---|
| B1 | 管理员解锁账户 | `clear_login_failures` **唯一调用点在登录成功分支**（`service/auth.rs:331`）。用户被锁只能干等 `LOGIN_FAILURE_WINDOW`（默认 300s）自然过期，管理员**无手动解锁入口** | `service/auth.rs:331`、`config/mod.rs:255` |
| B2 | 在线会话列举 + 单会话吊销 | 登录成功后**不写任何会话记录**，jti 只在登出时进黑名单。Redis 已有 `revoke_user_sessions`（整用户吊销）和 `delete_by_prefix`，**缺的是"谁在线"的数据**。JWT Claims 里 `jti` 早就有了（`utils/jwt.rs:22`），且有测试钉住"同一用户不同令牌 jti 唯一"——地基是齐的 | `service/auth.rs:358-369`、`utils/redis.rs:136-170`、`utils/jwt.rs:22` |
| B3 | 批量导入用户（CSV） | 只有 `GET /api/admin/export/users` 一个方向，没有反向。全仓 `import`/`导入` 只命中 TS 动态 import，无业务导入 | `controller/` 无导入端点 |

### 执行顺序（依赖驱动，不是按价值排）

```
M0  发布元数据对齐（版号/tag/CHANGELOG）        ← 先做，独立、低风险
M1  迁移 014：users 加 display_name / avatar_url
M2  A1 profile 端点 + A2 列表筛选              ← 可与 M3 并行
M3  B1 解锁端点 + 新权限码 system:user:unlock
M4  B2 会话登记（登录时写 Redis）+ 列举/单吊销端点 + system:session:manage
M5  A3 头像上传：开 axum multipart + tower-http fs + 落盘 + 静态路由
M6  前端串联 + e2e + 缺陷注入验证 + 门禁
```

M2 和 M3 无依赖，可并行。M4 依赖新的权限码种子机制，M5 依赖 M1 的 `avatar_url` 列。
**M5 放最后**：它是唯一一个动基础设施（开 feature、改 Dockerfile、可能要挂卷），
放前面会让 M1–M4 的回归定位变难。

### 关键风险与设计决定

**头像存储是本版最大的不确定项。** 三个选项：
1. 本地磁盘 + `tower-http` 的 `ServeDir`——最省事，但容器里需要挂卷，删容器会丢图；
2. 对象存储（S3/OSS）——生产正确，但本仓无任何对象存储依赖，引入 SDK + 配置项会明显扩大面；
3. 只存 URL 不做上传——最轻，但 `BaseUpload.vue` 还是悬空着。

**建议本版走 1**，把存储抽象留到 v0.22.0。理由：这是 admin 后台不是社交产品，头像丢了可接受；
而选项 2 会在本版塞进一套与"账号自持"主线无关的配置面。**必须配套**：
上传大小与 MIME 白名单、随机化文件名（**绝不能用原始文件名拼路径**）、Docker 挂卷说明。

**会话登记要留意存储增长。** 每个活跃令牌一条 `sess:*` key，必须靠 JWT 自身 TTL 自过期。
一旦漏了这个约束，Redis 会被永不登出的令牌撑爆。`revoke_user_sessions` 的时间戳机制
（`user_revoked_before`）是**整用户粒度**的，单会话吊销仍要走 jti 黑名单——两条路径不能混。

**新权限码必须同步种子。** `model/permission.rs` 是 const 单一数据源，驱动种子插入。
`system:user:unlock` 和 `system:session:manage` 要在这里加，否则 admin 的种子授权会漏。

### 验收线

- `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` 0 warning
- `cargo test --all-targets` + 集成 `133 passed`（会因新增用例上升）
- `pnpm lint` / `typecheck` / `test` / `build`
- **每个修复都要做缺陷注入**：注入后必须红，红完回滚，并查库清残留
  （注入型红测必然留脏数据，这是本仓已确认的规律）
- 新 admin handler 会被 `every_admin_handler_declares_a_permission_guard` 自动纳入；
  新端点会被 `every_documented_endpoint_is_reachable_without_a_server_error`（从 openapi 派生）自动纳入。
  **不要为了绕过这两条测试而改测试。**

### 一处必须先问的取舍

v0.19.0 的 tag / CHANGELOG / 版号**要不要补**。补的话有两种做法：
只补 tag（快，但 CHANGELOG 仍缺 0.19.0），或连 CHANGELOG 与版号一起抬到 0.20.0（一致，但 0.19.0 在 tag 历史里就成了没有条目的空档）。
倾向后者。**开工前需要你定。**

---

## 2. v0.21.0 —— 组织结构与系统参数

**主题**：从"用户是扁平的"走向"用户属于某个组织，系统行为可配置"。

| 编号 | 功能 | 说明 |
|---|---|---|
| C1 | 部门 / 组织树 | 全仓 grep 零命中。`users` 与 `roles` 之间没有中间层。带来新表 + `users.dept_id` + 树形端点 + 递归删除策略 + 前端树组件。工作量是本路线里最大的单块 |
| C2 | 系统参数配置表 | 全仓零命中。当前所有可调项都在 `config/mod.rs` 从环境变量读，改一个登录窗口要重启进程。做成运行时可改 + 缓存，是 B1 解锁策略、后续口令策略的共同底座 |
| C3 | 口令复杂度与过期策略 | 依赖 C2。当前 `utils/validation.rs` 只管用户名/邮箱的形状，不管口令强度 |
| C4 | 2FA / TOTP | 全仓零命中。需要引入加密 crate + 密钥保管 + 恢复码 + 前端扫码。**依赖 C2**（开关与策略要可配）。这是独立且用户感知强的大功能，建议单独占一版而不是塞进 v0.21.0 |

**排序理由**：C2 是 C3、C4 的前提，所以 C2 必须先做。C1 独立但量大，
可以和 C2 并行，但**不要同版**——树形递归删除的边界情况（父子循环、跨部门角色授权）
需要独立的测试预算。

---

## 3. v0.22.0 —— 规模化与通知

| 编号 | 功能 | 说明 |
|---|---|---|
| D1 | 邮件 / 通知通道 | **这是 A 线里"改邮箱"一直被搁置的原因**：没有验证通道，改邮箱就无法确认归属，只能靠管理员。补上通道后，"用户自助改邮箱"才能做对 |
| D2 | 用户自助改邮箱 | 依赖 D1 的验证码流程。本仓目前完全没有邮件依赖 |
| D3 | 头像存储抽象（切对象存储） | 兑现 v0.20.0 留下的技术债，把本地磁盘换成 S3/OSS |
| D4 | 审计日志导出 / 更细检索 | `admin/audit` 目前只有 2 个端点，而 `utils/audit.rs` 已经能产出 `diff_summary`、`permission_change` 这类明细——**能力已有，接口没暴露** |

**D4 是性价比最高的一项**：后端明细能力已经写好了，只差端点与前端检索页。

---

## 4. 已确认**不是**缺口（避免重复排查）

这几项在过往会话里被反复怀疑过，实际已闭环：

- **前端 GET 缓存无用户维度**。`cache.ts:97` 的 `buildKey` 确实只有 `method:url:params`，
  但 `stores/user.ts` 在登录成功后、`clearLocalSession` 内、401 拦截、任何非 GET 响应之后
  都调了 `requestCache.invalidate()`。已闭环，不是活漏洞。
- **审计只记写操作**。不成立——`audit_log_middleware` 对所有方法都写 `action = "{method} {path}"`。
- **"记住密码"存明文**。`login/index.vue:174` 的 `saveRemembered()` 只存 `{ username }`，已不存密码。
- **分页校验 / SQL 注入 / 级联删除**。分别由 `validate_page`、`get_order_sql()` 的
  `allowed_fields` 白名单、外键约束覆盖，均已确认扎实。
- **并发写后写胜出**。无乐观锁，v0.19.0 已判定可接受，不动。

---

## 5. 一页速查

| 版本 | 主题 | 核心项 | 量级 |
|---|---|---|---|
| **v0.20.0** | 账号自持 + 管理可应急 | profile 端点、列表筛选、解锁、在线会话、头像上传、批量导入 | 中 |
| v0.21.0 | 组织与配置 | 部门树、系统参数表、口令策略、(2FA 另占一版) | 大 |
| v0.22.0 | 规模化与通知 | 邮件通道、自助改邮箱、头像存储抽象、审计导出 | 中 |

**v0.20.0 的取舍主线**：优先补"用户和管理员各自缺的那只手"（自助改资料、管理员手动解锁与踢下线），
这些都在已有机制上闭环，风险可控；把动基础设施的头像上传放最后；把需要外部依赖的邮件通道、2FA 推到后面。
