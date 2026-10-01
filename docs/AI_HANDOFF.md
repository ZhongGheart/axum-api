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
| 8 | 文档：CHANGELOG / README / 版本号 | ✅ `42338986` |
| 9 | Chrome 端到端验证 | ✅ 12/12 通过 |
| 10 | 合并/tag/Release | ✅ 已发布 v0.4.0 |

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

- ✅ **12/12 通过**（真实 Chrome headless + CDP 直连，Node 22 内置 WebSocket，零依赖）。
  截图 `/tmp/e2e-{admin-full,admin-revoked,plain-user,plain-user-404}.png` 已人工复核。

### 上一个会话遗留的"阻塞点"其实是两个 bug，都已定位

1. **模板字符串里的 `\s` 不是正则转义**。上一会话把匹配器改成
    `textContent.replace(/\s/g,'')`，但这段代码在 JS **模板字符串**里，`\s` 是无效转义、
    会被折叠成字母 `s`，页面实际执行的是 `replace(/s/g,'')` —— 删掉的是字母而不是空格，
    `"登 录"` 因此永远匹配不上 `"登录"`。探针脚本能跑通只是因为它写成了 `\\s`。
2. `JSON.stringify` 把 SQL 里的真实换行转义成字面量 `\n`，psql 不会还原 → 需先压掉换行。

另外把 `waitFor` 改成**超时即抛错**（原来只记一条失败就继续跑，导致后续断言全建立在
"页面根本没渲染"之上，刷出一堆误导性失败项），并给 Vite 冷启动留足 45s。

## 升级路径验证（v0.3 → v0.4，真实二进制）

集成测试只覆盖空库启动，**没覆盖存量库升级**——这是真实用户最可能走的路径，
因此单独用 v0.3.0 的真实二进制做了验证：

1. `git worktree` 检出 v0.3.0 并编译 → 启动，产出**真实 v0.3 库**
   （迁移 001–006、14 条菜单、0 条 button 行、0 个 permission 值）
2. 用 v0.4.0 二进制启动同一个库

结论：

- 迁移 `007` 正常应用；新增 28 条 button 行，**原有 14 条菜单逐字节零改动**（ID 也保持）
- admin 自动获得全部 28 个权限码，侧栏 14 个菜单一个不少
- 全部 admin 接口 200，**无 403 回归**；`/api/admin/menus` 可见 28 个按钮节点
- 撤销的权限码重启后不会被重新授予（已单独实测）

## 全量回归结果（对照 CI 命令）

| 检查 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | ✅ |
| `cargo test --locked --all-targets --all-features` | ✅ 32 passed |
| 集成测试（fresh DB，`--ignored --test-threads=1`） | ✅ 16 passed |
| `pnpm install --frozen-lockfile` | ✅ 版本号变更不影响 lockfile |
| `pnpm lint` | ✅ 0 errors（`env.d.ts` 1 个历史 warning） |
| `pnpm typecheck` / `pnpm test` / `pnpm build` | ✅ 36 passed |

> 注：`pnpm-lock.yaml` 不记录项目版本号，故 `package.json` 改版本无需重新生成 lockfile。

## ✅ v0.4.0 已发布（2026-10-01）

- PR：https://github.com/ZhongGheart/axum-api/pull/3 （CI 三个 job 全绿后合并）
- 合并方式：merge commit `b4711d67`（保留 5 个提交的历史，未 squash）
- tag：`v0.4.0`（annotated，指向 merge commit）
- Release：https://github.com/ZhongGheart/axum-api/releases/tag/v0.4.0
- 已删除已合并分支 `v0.4.0`（本地 + 远程），master CI 复核全绿

### 发版流程里踩到的两个 `gh` 版本差异（下次直接照抄）

1. `gh pr merge` 用 **`-t/--subject`** 给 merge commit 标题，**没有 `--title`**；
   body 用 `-b/--body`。
2. `gh release create` 用 **`--notes-file`**，**没有 `--body-file`**；
   配合 `--verify-tag` 确保 tag 已在远端。
3. 删远程分支时 `git push origin --delete v0.4.0` 会报
   `dst refspec matches more than one`（分支与 tag 同名），
   必须写全 refspec：`git push origin :refs/heads/v0.4.0`。

## 当前状态（交接给下一轮）

- 分支：master，工作区干净，HEAD = `b4711d67`（= tag v0.4.0）
- 本地仍可能残留：E2E 后端 `:8080`、Vite `:5173`、`scripts/test_env.sh`
  的 Postgres/Redis（55432 / 56379）。收尾时用
  `scripts/test_env.sh stop` 与 `pkill -f "target/debug/axum-api"` 清理。
- 下一版候选方向：把 `require_role("admin")` 粗粒度闸门与权限码体系合并
  （当前非 admin 角色仍被整体挡住，权限码只细化 admin 路由）。

## 已知残留限制（本版不解决，需诚实记录）

- 权限码只细化 admin 路由；非 admin 角色仍被 `require_role("admin")` 整体挡住，
  暂不能只凭权限码访问管理接口。
- `role='admin'` 仍是粗粒度超级判断，未与权限码体系合并。

## 环境事实

- 本机 **Docker daemon 不可用**（`docker info` 失败）→ `docker build` / `compose up`
  端到端只能交给 CI；本地用 `scripts/test_env.sh`（真实 Postgres + Redis，无 Docker）验证。
- Chrome 存在；**未安装 Playwright**（全局与 `~/Library/Caches/ms-playwright` 均为空）。
  端到端验证需先解决驱动来源（Chrome CDP 直连或安装 Playwright chromium）。

---

# v0.5.0 — 角色与授权闭环

## 当前目标

**PR-1：让"按角色分配权限码"这件事真正可用。**

v0.4.0 把权限码变成了强制鉴权，但**没有任何一条正常路径能把权限码授给 admin 以外的角色**。
这是一条四环死链（PR-1 打通前三环，第四环留给 PR-3）：

| 环节 | 位置 | 状态 |
|---|---|---|
| 角色管理页只读（63 行表格，无增删改、无授权入口） | `frontend/src/views/system/role/index.vue` | ❌ PR-1 修 |
| 前端从未调用 `PUT /roles/{id}/menus` | `frontend/src/api/role.ts` | ❌ PR-1 修 |
| 后端拒绝分配自定义角色（400） | `src/controller/user.rs:25,57-66` `ASSIGNABLE_ROLES` | ⏸ PR-2 |
| `require_role("admin")` 整体挡住非 admin 角色 | `src/router/mod.rs` 5 处 | ⏸ PR-3 |

## 动手前发现的缺陷（必须先修，否则不能接线 UI）

**授权写路径用 `.ok()` 吞掉所有错误。** `src/repository/menu.rs` 与 `src/controller/role.rs`
共 6 处 `.ok()`，模式一致：`BEGIN` → 吞错的 DELETE/INSERT → `COMMIT`。

1. **`assign_role_menus`（`src/repository/menu.rs:253-267`）——安全相关。**
   DELETE 的 `.ok()` 吞错后事务照样提交 → **撤销权限码可能静默失败**
   （取消勾选 → 保存 → 权限还在）。INSERT 的 `.ok()` 吞错后循环继续 →
   传入"合法 + 非法"混合 ID 时**静默部分授权**，却返回"权限分配成功"。
   这个函数实际上永远不会因业务失败而失败。
2. **`delete_role`（`src/controller/role.rs:186-197`）。** 吞掉 `user_roles` 删除错误；
   两条语句非事务；**无任何守卫**——可以把 `admin` 角色删掉，
   或删掉仍被大量用户持有的角色（FK `ON DELETE CASCADE` 会静默让这些用户失去角色）。
3. **`delete_menu` 递归删除（`src/repository/menu.rs:198-224`）。** 子节点删除吞错 +
   父节点照样提交 → 留下 `parent_id` 指向已删父节点的孤儿菜单
   （导航树里不可见的脏数据）。

> 结论：**先修吞错，再接 UI。** 否则"取消勾选后保存"这种最普通的操作会静默失效，
> 而 UI 会让人以为撤销成功了。

## 当前计划

| 步骤 | 内容 | 状态 |
|---|---|---|
| 0 | 建立 `v0.5.0` 分支、记录计划 | ✅ |
| 1 | 修 `assign_role_menus`：去 `.ok()`，错误上抛，保持单事务 | ✅ `cc582550` |
| 2 | 修 `delete_role`：单事务 + 禁止删内置 admin + 报告影响用户数 | ✅ `cc582550` |
| 3 | 修 `delete_menu` 递归吞错（同一根因，避免只修一半） | ✅ `cc582550` |
| 4 | 后端集成测试：撤销生效 / 非法 ID 报错 / 拒绝删内置角色 | ✅ `cc582550`（16→25） |
| 5 | 前端 `roleApi` 补 create/update/delete/assignMenus | ✅ `1d1e0887` |
| 6 | 角色管理页可写 + 菜单/权限码授权树 | ✅ `1d1e0887` |
| 7 | 新按钮按权限码 gate（`system:role:*`、`system:menu:grant`） | ✅ `1d1e0887` |
| 8 | 前端测试 + 全量回归 | ✅ 69 前端 / 36 单测 / 26 集成 / 36 E2E |
| 8.5 | 接 UI 时发现并修掉 v0.4.0 遗留的 `?role_id=` 空树缺陷 | ✅ `aa532547` |
| 9 | 文档（CHANGELOG / README / 版本号） | ✅ 版本号 0.4.0 → 0.5.0 |

## 起始 git 状态

- 分支：`v0.5.0`（从 master `2a3443e7` 切出）
- 工作区：干净
- v0.4.0 已发布：https://github.com/ZhongGheart/axum-api/releases/tag/v0.4.0

## 关键设计决定

1. **PR-1 不碰安全语义**。`ASSIGNABLE_ROLES` 与 `require_role("admin")` 分属 PR-2/PR-3，
   本 PR 保持原样——只把"已有后端能力"接上 UI 并修好它的写路径。
2. **授权写路径要么完整成功、要么整体失败**。半吊子的授权比没有授权更危险，
   因为用户会以为撤销生效了。
3. **拒绝删除内置 `admin` 角色**：与既有 `ensure_not_last_admin`（防止最后一名管理员被降级）
   属同一类保护——防止把系统改造成无人能管理的状态。
4. 授权树直接用 `GET /api/admin/menus`（`find_tree()` 不做类型过滤，返回含 button 节点），
   数据已现成，无需新接口。

### 步骤 5-7 前端（动手前记录）

5. **授权弹窗用「全量菜单树」渲染，只把 `listByRole()` 的结果当默认勾选值**。
    不用 `listByRole()` 的树本身渲染：它经 `build_tree(filtered, None)` 后，
    父节点未被授权的子节点会**上浮成根节点**，父子关系与真实结构不一致；
    此时 naive-ui `cascade` 勾选一个菜单页会连带勾上"恰好已授权"的子孙按钮 →
    **静默扩权**。用全量树则 cascade 语义稳定，提交的是完整勾选集合，
    既不静默扩权也不静默丢授权。
6. **纯逻辑抽到 `src/utils/menu.ts`**（`flattenMenuIds` / `buildGrantTreeOptions`），
    用 vitest 直接覆盖。前端没有 `@vue/test-utils`，组件内部状态无法单测，
    因此把值得测的部分移出 `.vue`。
7. **内置角色名不在前端另立定义**：`constants/builtin.ts` 的 `BUILTIN_ROLE_NAMES`
    由契约测试对着后端 `model/role.rs` 的 `BUILTIN_ROLES` 校验（含 `ADMIN_ROLE`
    必须在集合内）。内置角色直接不渲染删除按钮——一个必然 400 的按钮不该存在。
8. **顺带修 `views/system/menu/index.vue` 的 ungated 按钮**（v0.4.0 遗留）：
    新增/编辑/删除三个入口都没接权限码，等于权限码体系在菜单页自己身上漏了。
    同属"权限码闭环"，一并修而不是留着。

> 步骤 5-7 涉及的受影响文件：`frontend/src/api/role.ts`、`frontend/src/api/menu.ts`、
> `frontend/src/utils/menu.ts`、`frontend/src/constants/builtin.ts`、
> `frontend/src/views/system/role/index.vue`、`frontend/src/views/system/menu/index.vue`。
> 预期下一步：前端单测覆盖上述纯逻辑与 API 契约，再做全量回归。

### 步骤 5-8 进展日志

#### `1d1e0887` feat(role): 角色管理页可写 + 菜单/权限码授权树

- `roleApi` 补 `create/update/delete/assignMenus`（后端早已就绪，前端无入口）。
- 角色页从 63 行只读表格改为可写：增删改 + 授权树（含 `type='button'` 权限码节点），
  按钮按 `system:role:*` 与 `system:menu:grant` gate。
- 内置角色（admin/user）不渲染删除按钮——后端必然 400。前端名单
  `constants/builtin.ts` 由 `builtinRoles.spec.ts` 对着后端 `BUILTIN_ROLES` 校验。
- 顺带修 v0.4.0 遗留缺口：菜单页三个行内入口与「新增根菜单」此前完全没接权限码。
- 前端测试 36 → 69（+33）。注入缺陷验证：去掉 ID 过滤、前端名单多加一个角色，各自失败。

#### `aa532547` fix(menu): 按角色查菜单树不再丢弃父节点未授权的节点

**v0.4.0 就存在的缺陷，只是从没人调过 `GET /api/admin/menus?role_id=`。**
接授权弹窗时实测：给一个只被授予「角色管理 + 其 4 个按钮」的角色，
`role_menus` 有 5 行，接口却返回 `[]`。

- 根因：`build_tree(filtered, None)` 只把 `parent_id IS NULL` 当根，
  父节点不在过滤结果里的节点被**静默丢弃**。
- admin 恰好看不出问题：种子把它授满全部 42 个菜单，所有祖先都在集合里。
- 危害：授权弹窗对部分授权的角色显示「该角色没有任何权限」，
  管理员一保存就按**全量覆盖**把授权清空——静默的数据丢失。
- 改成森林语义（父不在集合内即视为根）。同一函数也服务 `/api/auth/menus`，
  因此「授权了菜单页但没授权其上级目录」的用户，之前侧栏里根本看不到那个页面，
  现在会以顶层出现。
- 测试 +4 单元 / +1 集成（25→26）。**注入缺陷验证**：改回旧语义后，
  集成测试报 `只授权一个菜单时应只返回它，实际: []`，2 个单元测试同时失败。

> 教训：交接文档里「授权树数据已现成，无需新接口」的判断是**只看了代码、没跑数据**。
> 下次给类似结论前，先用 curl 打一次真实响应。

#### naive-ui `n-tree` 的 DOM 结构（写 E2E 时踩的坑）

```
.n-tree-node
  ├ .n-tree-node-indent
  ├ .n-tree-node-switcher
  ├ .n-tree-node-checkbox        ← 复选框外层
  │   └ .n-checkbox[role=checkbox][aria-checked]
  └ .n-tree-node-content         ← 标签文本，与复选框是**兄弟**不是后代
```

用 `row.querySelector('.n-checkbox')`（row 取 content）会拿到 `null`。
勾选状态读 `aria-checked` 比 `classList.contains('n-checkbox--checked')` 稳。
另：关闭的 `n-modal` 仍留在 DOM 里（`display:none`），
要按**内容特征**定位弹窗，不能靠出现顺序。

#### `cascade` 的半选语义（授权弹窗的真实行为）

勾选父菜单会连带勾上全部子孙（符合预期）。但**取消一个子按钮**后，
父菜单变为半选（indeterminate），就不在 `checked-keys` 里了 →
保存时父菜单的授权也被撤销。所以「取消 1 个按钮」实际让 `role_menus`
从 5 行减到 3 行。这是 naive-ui 的既定语义、UI 也如实显示了半选，
但容易被误解，已在弹窗提示里写明。

#### E2E 结果（`/tmp/e2e-role.mjs` + `/tmp/e2e-lib.mjs`）

- ✅ **36/36 通过**（真实 Chrome headless + CDP 直连，Node 22 内置 WebSocket，零依赖）。
- 覆盖：内置角色无删除按钮、UI 新建角色落库、授权树展示权限码、
  cascade 勾父带子（且不波及无关模块）、保存后 DB 落库、**重新打开正确回显**、
  取消勾选后 DB 真的撤销、半选语义、撤销 `system:role:create` 后按钮从 DOM 移除
  且接口 403、UI 删除角色后 `role_menus` 级联清理无残留。
- 截图 `/tmp/e2e-role-0*.png` 已人工复核。
- 起服务注意：后台进程会随 shell 退出被回收，必须用长驻 session 跑
  `./target/debug/axum-api` 与 `pnpm dev`；`pnpm dev` 需 `--host 127.0.0.1`。

## 后续版本候选

- `system:monitor:export`、`system:test:access` 后端有码但前端无入口——按"要么接线要么删除"
  的既有原则二选一，别悬着
- `api_metrics` 仍在进程内 `Arc<RwLock<HashMap>>`：重启丢失、多副本不聚合
- `audit_logs` 无保留/清理策略
- Prettier 未进 CI

## E2E 脚本备忘

上一会话的 `/tmp/e2e-perm.mjs` 用 Node 22 内置 `WebSocket` 直连 Chrome CDP（零依赖）。
踩过的坑：**模板字符串里的 `\s` 不是正则转义**，会被折叠成字母 `s`，
要写成 `\\s`；`JSON.stringify` 会把 SQL 换行转义成字面量 `\n`，需先压掉换行。
