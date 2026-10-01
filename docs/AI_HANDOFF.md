# AI_HANDOFF — 崩溃恢复日志

本文件是跨会话的交接日志。任何非平凡改动在**动手前**先写这里，达成里程碑后更新。
接手者必须先把它与 `git status` / `git diff` / 实际文件系统对账。

## 当前目标

发布 **v0.4.0：把 `menus.permission` 从元数据变成真正的权限码**。

上一版（v0.3.0）的"已知限制"里排第一的是：

> 按钮级权限仍为基于角色的显隐；`menus.permission` 目前只是元数据，未接入接口级权限码校验

本版目标：**接口级权限码校验**。

- `menus.permission` 承载权限码（`type='button'` 的菜单行），经 `role_menus` 授权
- 后端：新增权限码单一数据源 + `PermissionGuard` 提取器 + `/api/auth/permissions`
  - 所有 `/api/admin/*` handler 显式 `require(权限码)`
- 前端：`v-permission` / `PermissionButton` 改为按权限码判定，不再按角色
- 测试：证明"权限码真的在拦截"（撤销授权后 403），而非仅是元数据

## 当前计划

| 步骤 | 内容 | 状态 |
|---|---|---|
| 0 | 接手 v0.3.0 会话、重建上下文 | ✅ |
| 1 | `src/model/permission.rs`：权限码单一数据源（const 定义 + 元数据表） | ✅ `b4de0d0a` |
| 2 | `MenuRepository::find_permission_codes(role_names)` | ✅ `b4de0d0a` |
| 3 | `PermissionGuard` 提取器 + 全量 `/api/admin/*` handler 接入 | ✅ `b4de0d0a` + `a65ab453` |
| 4 | 权限码种子（按钮型菜单）+ admin 授权（幂等 upsert） | ✅ `b4de0d0a` |
| 5 | `GET /api/auth/permissions` | ✅ `b4de0d0a` |
| 6 | 前端：权限码 store / 指令 / `PermissionButton` / 调用点 | ✅ `a65ab453` |
| 7 | 测试：后端集成 + 前端单测 + 契约测试 | ✅ `b4de0d0a` / `a65ab453` |
| 8 | 文档：CHANGELOG / README / 版本号 | ⬜ |
| 9 | Chrome 端到端验证 + 合并/tag/Release | 🔄 E2E 进行中 |

## 起始 git 状态

- 分支：master（工作区干净）
- HEAD：`13f7c07d Merge pull request #2 from ZhongGheart/v0.3.0`
- tag：`v0.1.0` / `v0.2.0` / `v0.3.0`

上一会话 `01a0acab-3a00-7603-b7b3-da23d191542d` 已完成 v0.3.0 合并 + tag + Release
（https://github.com/ZhongGheart/axum-api/releases/tag/v0.3.0），
最后一个 turn 因 HTTP 429 失败，工作内容已全部落盘。

## 关键设计决定

1. **权限码存 `menus.permission`，不新建权限表**。`type='button'` 的菜单行即权限码，
   复用既有 `role_menus` 授权与"菜单管理"页面；不引入第二套授权数据源。
2. **权限码单一数据源**：`src/model/permission.rs` 的 const 字符串既被 handler 引用，
   又驱动种子插入。种子由 const 列表循环生成，不再手写大段 SQL 字符串，杜绝两处漂移。
3. **admin 是数据上的超级用户，不是代码里的后门**：`require_role("admin")` 仍是粗粒度闸门，
   权限码是细粒度闸门（AND 语义）。admin 靠种子获得全部按钮权限，可被显式撤销。
4. **不做权限码缓存**：每次受保护请求一次索引 JOIN。撤销权限立即生效，
   避免 TTL 窗口内的越权。可观测成本留给后续按需优化。
5. **契约测试**：源码扫描断言"每个 `/api/admin/*` handler 都调用了 `require(`"，
   防止将来新增管理接口漏接权限码（沿用 v0.3.0 文档双向覆盖测试的思路）。

## 进展日志

### `b4de0d0a` feat(rbac): 权限码成为接口级强制授权

- 新增 `AppError::PermissionDenied(String)`，403 响应体带缺失权限码。
- 迁移 `migrations/007_permission_codes_unique.sql`：`menus.permission` 部分唯一索引。
- `seed_navigation` 拆分：菜单树"仅 menus 为空才写"，权限码**无条件幂等补齐**
  （v0.3 升级库 menus 非空但无按钮行）。**只对新���建的权限码行授予 admin**
  ——否则每次启动补授权会把管理员被显式撤销的权限悄悄恢复。已实测：撤销后重启，撤销保持。
- `get_items_by_code`（`/api/dict/{code}/items`）**刻意不设权限码**：普通页面的
  DictSelect 依赖它，加了会让非管理员的字典下拉全部失效。

### `a65ab453` fix(rbac): 权限码校验前移到提取阶段

自查发现的层级缺陷：校验原先写在 handler 函数体内，而 axum 先执行 `Json<T>` 提取，
导致"无权限 + 畸形请求体"返回 **422 而非 403** ——等于把接口参数结构反馈给了无权限方。

- 新增 `src/middleware/permission.rs`：宏 `permission_guards!` 生成类型化提取器
  （`PermUserList` 等），`impl FromRequestParts<AppState>`，在**提取阶段**校验。
- 38 个 handler 签名改为 `_perm: PermUserList,`，函数体内不再出现 `perm.require(...)`。
- 注意：前端改动也落在本 commit（工作区当时并不脏，已随本 commit 提交）。

## 测试现状（截至 `a65ab453`）

- `cargo fmt --check` / `cargo clippy -D warnings`：clean。
- 后端单测 32 passed；集成测试 16 passed（真实 Postgres + Redis）。
  关键用例：撤销后立即 403、鉴权早于入参校验（403 而非 422）、
  授权接口自身受 `system:menu:grant` 保护、`every_admin_handler_declares_a_permission_guard`
  源码契约测试（**已注入缺陷验证其真的会失败**）。
- 前端：typecheck clean、`pnpm test` 36 passed、lint 0 errors（`env.d.ts` 1 个历史 warning）、
  `pnpm build` 通过。前后端权限码契约测试含 `BACKEND_ONLY_CODES` 豁免。

## 当前 E2E 状态

- 后端 `:8080`（库 `axum_api_e2e`）+ Vite `:5173` 仍在运行（上一会话遗留的 TTY session）。
- curl 层已验证：登录 OK、admin 拿 28 个码、撤销 `system:user:delete` 后变 27、
  `GET /api/admin/roles` 不受影响。
- **阻塞点**：`/tmp/e2e-perm.mjs` 的 `waitFor` 在 `Page.navigate` 之后疑似处于失效的执行上下文。
  探针脚本 `/tmp/probe.mjs`（固定 sleep 3.5s）能稳定读到按钮 `["登 录"]`，
  说明是脚本等待逻辑问题，不是页面渲染问题。

## 已知残留限制（本版不解决，需诚实记录）

- 权限码只细化 admin 路由；非 admin 角色仍被 `require_role("admin")` 整体挡住，
  暂不能只凭权限码访问管理接口。
- `role='admin'` 仍是粗粒度超级判断，未与权限码体系合并。

## 环境事实

- 本机 **Docker daemon 不可用**（`docker info` 失败）→ `docker build` / `compose up`
  端到端只能交给 CI；本地用 `scripts/test_env.sh`（真实 Postgres + Redis，无 Docker）验证。
- Chrome 存在；**未安装 Playwright**（全局与 `~/Library/Caches/ms-playwright` 均为空）。
  端到端验证需先解决驱动来源（Chrome CDP 直连或安装 Playwright chromium）。
