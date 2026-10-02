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
  （v0.3 升级库 menus 非空但无按钮行）。**只对新建的权限码行授予 admin**
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

# v0.5.0 PR-2 — 拆 `ASSIGNABLE_ROLES`（角色成为一等数据）

## 当前目标

**让"角色"从硬编码常量变成真正的数据。** PR-1 打通的前三环里，剩下的一环是
`src/controller/user.rs` 的 `ASSIGNABLE_ROLES`（`["admin","user"]`）：
后端拒绝分配任何自定义角色，用户表单的角色下拉在前端也是写死的两个选项。

做完 PR-2，PR-1 建立的自定义角色才真正能被用起来——**否则 PR-1 的授权树
只对 admin 有效，自定义角色是一个永远没人能创建的用户类型的空壳。**

## 动手前的调查结论（决定了下面的做法）

1. **白名单下游已经有存在性校验**：`RoleRepository::replace_user_roles`
   在事务内逐个 `SELECT id FROM roles WHERE name = $1`，不存在则
   `NotFound("角色不存在: {name}")`。所以"查库校验"不需要新增查询，
   白名单只是**提前**拦截，且抢先于仓库层的 404 变成了误导性的 400
   （"角色必须是 admin / user 之一"）。
2. **拆掉白名单会立刻暴露一个既有缺陷**：`create_user` 先
   `user_repo.create(...)` 建好用户，**再** `replace_user_roles` 校验角色，
   两者不在同一事务。白名单在前面挡着，非法角色永远到不了这一步；
   一旦拆掉，`role=ghost` 会返回 404，而**用户行已经落库**，
   留下一个没有任何角色的"半成品用户"，前端提示失败、库里却多了一条记录。
   → 必须把角色校验提到写用户**之前**（`update_user` 已经有
   "先做守卫，避免基础字段已更新但角色变更被拒绝"的同类注释，方向一致）。
3. **角色名没有统一归一化，这是拆白名单后的隐性陷阱**：`create_role` /
   `update_role` 对 `name` **零校验**（不 trim、不限长、不查重），
   而用户表单路径一直 `trim().to_lowercase()`。于是可以建出角色 `Auditor`，
   下拉里选得到它、提交后端归一成 `auditor` → 查不到 → 400
   "角色不存在"。**角色是拿名字当授权键用的**（权限码按 `role_menus.role_id`
   授权、按角色名匹配），两份名字对不上就是静默的授权错配。
   → 角色名在**写入时就归一化**，`roles` 表里的名字一律 canonical。
4. **存量数据必须迁移**：v0.5.0 PR-1 的角色管理页已经能建角色，
   生产/开发库里可能已有 `Auditor` 这类大小写混杂的角色。不迁移的话
   PR-2 上线即"这些角色突然不能分配了"。沿用 v0.4.0 的升级路径纪律，
   加迁移并做真实升级验证。
5. **重命名内置角色是同一类漏洞**（PR-1 修了 `delete_role`，漏了 `update_role`）：
   把 `admin` 改名后，`ADMIN_ROLE = "admin"` 的查找（最后一名管理员保护
   `count_users_with_role`、权限码种子 `WHERE r.name='admin'`）全部落空，
   系统会变成"没人是管理员"。反过来把自定义角色改名成 `admin` 也应拒绝。

## 关键设计决定

1. **归一化函数放 `model/role.rs`**（角色名策略的唯一数据源，与
   `BUILTIN_ROLES` 并排），create/update role、create/update user、
   `POST /users/:id/roles` 五处共用，**不允许各处各写一份**。
2. **归一化 = `trim().to_lowercase()` + 非空 + ≤50（对齐 `VARCHAR(50)`）
   + 不含空白/控制字符**。不做 `[a-z0-9_-]` 白名单式字符集限制：
   中文角色名在这个中文界面里是合理需求，限制它没有技术收益。
3. **内置角色既不可删除也不可改名，且不能被占用名字**——与 PR-1 的
   "内置角色不可删除"同一条约束的完整形式。
4. **`create_role` / `update_role` 撞名改为 409**，不再把 DB 唯一约束
   违例当成 500 吐给用户。归一化会让"建 Auditor 再建 auditor"变成常见操作，
   这条不是锦上添花。
5. **用户侧角色校验返回 400 并带上实际角色名**（不是 404、不是枚举提示）：
   这是一个表单字段的取值错误，400 语义正确，且不向调用方泄露角色清单。
6. **前端下拉由 `GET /admin/roles` 驱动**，选项拼装逻辑抽到纯函数并单测
   （沿用 PR-1 `utils/menu.ts` 的做法：组件内部状态没法单测，值得测的部分移出去）。
   默认选中值：优先 `user`，否则列表首个。

## 受影响文件

| 文件 | 改动 |
|---|---|
| `src/model/role.rs` | 新增 `normalize_role_name`；修正 `BUILTIN_ROLES` 文档（"允许分配给用户"这半句在 PR-2 后不再是该常量的属性） |
| `src/controller/user.rs` | 删 `ASSIGNABLE_ROLES`；`normalize_role` 改为"归一化 + 写前查库" |
| `src/controller/role.rs` | `create_role`/`update_role` 归一化 + 校验 + 内置角色改名守卫 + 409 |
| `src/controller/role.rs` | `assign_user_role` 也走归一化（否则同一次赋值在两个接口上一成功一 404） |
| `migrations/008_normalize_role_names.sql` | 存量角色名归一化（冲突行跳过而非迁移失败） |
| `frontend/src/views/system/user/index.vue` | 角色下拉改为读 `roleApi.list()` |
| `frontend/src/utils/role.ts`（新） | 下拉选项拼装纯函数 |
| `tests/api_integration.rs` | 自定义角色可分配 / 非法角色不落半成品用户 / 改名守卫 / 归一化 / 409 |

## 已知遗留（不在 PR-2 范围）

持有 `system:user:create` 的管理员可以建出 admin 用户——**创建用户即等于授予管理员**，
这是 v0.4.0 之前就有的提权面。修它需要"把 admin 作为可分配值"也纳入授权判断，
牵动 PR-3 的 `require_role("admin")` 改造，一并记在后续候选里，不在本 PR 顺手改。

## 步骤 10-14 进展日志

#### 后端：角色名成为单一数据源

> 成果 commit：`e20be0ed` feat(role): 拆掉 ASSIGNABLE_ROLES，角色从常量变成数据

- `model/role.rs` 新增 `normalize_role_name`（trim + 小写 + 非空 + ≤50 字符 + 无控制字符）。
  五处写入/取值路径共用它，不允许各处各写一份。
- 删掉 `controller/user.rs` 的 `ASSIGNABLE_ROLES`；`resolve_role` 改为
  **归一化 + 写用户之前查库**。
- `create_role` / `update_role` / `assign_user_role` 全部接入归一化；
  `update_role` 加事务 + `FOR UPDATE`，拒绝改名内置角色、拒绝占用内置角色名，
  撞名从 500 改为 409（`roles_name_key`，沿用 `repository/user.rs` 的既有约定）。
- `update_role` 改为**回读真实行**：原先返回编造的 `created_at = now()` 和
  `user_count = 0`——改一个 50 人角色也会回一个"0 人、刚创建"的角色。

#### 后端：拆白名单时发现并修掉的既有缺陷

**`create_user` 会留下半成品用户。** 它先 `user_repo.create(...)` 建用户行，
**再**调 `replace_user_roles` 校验角色，两者不在同一事务。白名单在前面挡着，
非法角色永远到不了这一步；拆掉后 `role=ghost` 会返回 404，而**用户行已经落库**。

注入缺陷实测（去掉 `resolve_role` 的查库）后查库确认：
`user_ghost_903774b1|0` ——一个 0 角色的用户静静躺在库里。
→ 角色校验必须提到写用户**之前**（`update_user` 早有同类守卫注释，方向一致）。

**重命名内置角色与删除它是同一类破坏，PR-1 只修了删除。**
`ADMIN_ROLE = "admin"` 是 `count_users_with_role`（最后一名管理员保护）和
权限码种子（`WHERE r.name='admin'`）的查找依据，改名后全部落空。
注入缺陷实测：`admin` 被改名 `superadmin` 时接口直接返回 200。

#### 前端：接 UI 时发现的静默数据损坏

**`UserInfo.role` 是只有 admin/user 两值的展示枚举**（`Role::primary_from`，
非 admin 一律塌缩成 user），真实角色集合在 `roles` 数组里。
用户页原本用 `user.role` 回填编辑表单和渲染角色列，PR-2 一旦允许自定义角色：
持有自定义角色的用户**一打开编辑框就变成"普通用户"，一保存角色就被改掉**。
→ 新增 `utils/role.ts` 的 `currentRoleName`，列表列与表单回显都改用真实角色集合。
这条靠集成测试看不出来（后端返回是对的），是 Chrome E2E 抓到的。

角色下拉改为读 `GET /admin/roles`，并**懒加载**（只在打开新建/编辑弹窗时拉）：
该接口需要 `system:role:list`，只浏览用户列表的账号不该被要求具备这个权限。

#### 迁移 `008_normalize_role_names.sql` 里踩的坑（改了三版才对）

1. 第一版 `lower(btrim(name))` 只裁普通空格，PR-1 建角色接口零校验，
   `E'  \t '` 这种名字能存进去，裁完仍是非法名。
2. 第二版 `btrim(name, '[:space:]')` —— **错的**。`btrim` 第二参数是**字符集合**，
   不是字符类，它逐字符匹配 `[ : s p a c e ]` 七个字符。
   实测把 `admin` 裁成 `dmin`、`auditor` 裁成 `uditor`。**角色名被改坏。**
3. 定稿用 `regexp_replace(name, '^[[:space:]]+|[[:space:]]+$', '', 'g')`。

附带两个陷阱：

- `sqlx::migrate!` 在**编译期**内嵌迁移文件。改完 `migrations/*.sql` 不重新
  `cargo build`，跑起来的还是旧迁移——我因此误判了一轮"迁移有 bug"。
- BSD sed 不支持 `\b`，`sed 's/\broles\b/.../'` 会静默不替换，
  测试脚本因此把迁移打到了**真实的 roles 表**上。改用 perl。

**迁移范围只含大小写与首尾空白**这两种静默错配；角色名中间有空格
（如 `senior auditor`）是合法的，空格能原样往返。
最初我把"中间有空格"也判为非法，结果迁移产出的名字**应用自己拒绝**——
存量角色会变成永远分配不了的死数据。

#### 升级路径验证（PR-1 真实二进制 → PR-2 真实二进制，同一个库）

沿用 v0.4.0 的做法，用 `git worktree` 检出 PR-1（`9fb5e359`）编译成真实二进制，
启动产出**真实 PR-1 库**（迁移 001–007），再用 PR-2 二进制启动同一个库。

先用 PR-1 二进制实测确认了缺陷前提是真的：它的建角色接口**真的**接受了
`"  Senior Auditor  "`，而它的用户表单对这个角色返回
`400 角色必须是 admin / user 之一`。

库内预置存量角色后升级，结果全部符合预期：

| 升级前 | 升级后 | 说明 |
|---|---|---|
| `  Senior Auditor  ` | `senior auditor` | 正常归一化 |
| `auditor` | `auditor` | 已 canonical，不动 |
| `Admin` | `Admin` | 归一化后与 `admin` 冲突，原样保留 |
| `  admin  ` | `  admin  ` | 同上 |
| `E'  \t '` | `E'  \t '` | 裁完为空串的死数据，原样保留让管理员看见 |

`role_menus=47 / menus=42 / users=1 / user_roles=2` 升级前后逐项一致，
迁移 1–8 全部应用，`/api/health` 正常。
升级后用 `"  Senior Auditor  "` 建用户 → **200**（PR-1 下是 400），
落库角色为 `senior auditor`。

#### 测试现状（PR-2 完成时）

- `cargo fmt --check` / `cargo clippy -D warnings`：clean。
- 后端单测 **44 passed**（36 → 44，新增 `normalize_role_name` 8 个）。
- 后端集成 **34 passed**（26 → 34）。新增 8 个用例：自定义角色可分配（建/改）、
  非法角色不留半成品用户、更新失败不改基础字段、归一化+409、内置角色改名守卫、
  `update_role` 回读真实行、追加角色接口归一化。
- 前端 **81 passed**（69 → 81，新增 `roleUtils.spec.ts` 12 个）；typecheck clean；
  lint 0 errors（`env.d.ts` 1 个历史 warning）；build 通过。
- Chrome E2E **9/9 通过**（`/tmp/e2e-pr2-role-assign.mjs`），截图
  `/tmp/e2e-role-pr2-0*.png` 已人工复核。
- **注入缺陷验证 2 处**：去掉角色存在性查库 → 半成品用户用例失败（并查库证实
  0 角色用户真实存在）；去掉内置角色改名守卫 → 守卫用例失败（admin 被改名）。
  两处均已还原并重跑全绿。

#### 接 UI 时的两个设计修正（原计划 → 实际）

1. 原计划"拒绝含空白的角色名"**过严**。角色名是 JSON 字符串，空格能原样往返，
   不构成"选得到却存不进去"的静默错配；而大小写和首尾空白才是。收紧后会让
   迁移产出的名字被应用自己拒绝，制造出一批死数据。改为只拒控制字符。
2. 下拉**懒加载**而非挂载即取：避免让"只浏览列表"的账号被迫需要 `system:role:list`。

### PR-2 遗留（不在本 PR 范围，已记入后续候选）

- 持有 `system:user:create` 的管理员可以建出 admin 用户——**创建用户即等于授予管理员**。
  这是 v0.4.0 之前就有的提权面，修它需要把"admin 作为可分配值"也纳入授权判断，
  与 PR-3 的 `require_role("admin")` 改造同批做。
- 用户表单是单角色语义（提交即**整体替换**角色集合），而 `POST /users/:id/roles`
  是追加语义。多角色用户经用户表单保存会丢掉其余角色——本 PR 沿用既有语义未改。
- `system:monitor:export` / `system:test:access` 仍是有码无入口。

## E2E 脚本备忘

上一会话的 `/tmp/e2e-perm.mjs` 用 Node 22 内置 `WebSocket` 直连 Chrome CDP（零依赖）。
踩过的坑：**模板字符串里的 `\s` 不是正则转义**，会被折叠成字母 `s`，
要写成 `\\s`；`JSON.stringify` 会把 SQL 换行转义成字面量 `\n`，需先压掉换行。

# v0.5.0 PR-3 — 撤掉 `require_role("admin")`（角色闸门降级为权限码）

## 当前目标

删掉 `src/router/mod.rs` 里 5 处 `require_role("admin")` 中间件，让"能进管理区"
完全由权限码决定，不再叠加一道角色硬闸门。

**前置核查（重做了上一轮那个错结论）**：上一轮用 `/tmp/check_guards.py` 正则扫描，
报"38 个 admin handler 里 36 个缺 `_perm: Perm`"。**这个结论是脚本 bug**——
签名跨多行且含嵌套括号，`[^)]*` 匹配不到。仓库里已有的契约测试
`every_admin_handler_declares_a_permission_guard`（`tests/api_integration.rs`，
非 `#[ignore]`，随 `cargo test --lib` 一起跑）用 `rfind("\npub async fn ")` 切函数体，
照抄它的切分逻辑重查后：

- 38 个 `/api/admin/*` handler，**38 个都有类型化守卫，0 个缺失**；
- 29 个权限码定义，**29 个都被某个守卫用到**，无有码无入口；
- 因此拆掉角色闸门不会暴露任何裸接口。

## 关键设计决定：提权面用"权限码包含关系"判定，不用新增权限码

拆闸门后真正的风险不是"接口没守卫"，而是 **AND 语义消失**：
此前 `require_role("admin")` 与权限码守卫是 AND，等于"角色是权限码的上游闸门"。
闸门一撤，持有 `system:user:create` 的自定义角色就能直接建出 **admin 用户**——
**创建用户即等于授予管理员**（PR-2 遗留里点名的那条）。

候选方案与取舍：

- (a) 新增权限码 `system:user:grant-admin`：语义最正，但要动种子 + 前端 PERM 常量 +
  文档 + 授权树，且"谁能授予管理员"本身又变成一个新的提权面（谁授予 grant-admin？）。
- (b) 窄角色检查（仅 admin 持有者可授予 admin）：改动小，但把 `admin` 重新写回代码，
  与 PR-2"角色不是常量"的方向相反。
- **(c) 采用：权限码包含关系（authority superset）**。

(c) 的规则：**你能授予的权限码，必须全部是你自己已持有的。**
即调用者的权限码集合 ⊇ 目标角色/目标用户的权限码集合。这条规则同时覆盖
建用户、改用户、追加角色、重置密码、停用、删除、角色授权菜单全部写路径，
且**不需要新权限码、不需要新增种子**，与既有 `find_permission_codes` 同源。

它同时天然堵住了三条原本没被点名的提权路径：

1. `create_user(role=admin)` — 建出管理员
2. `update_user(role=admin)` / `assign_user_role(admin)` — 追加语义同样能提权
3. `delete_user` / `toggle_user_status` / `reset_user_password` / `batch_delete` —
   目标若是权限比自己高的人（如 admin），这几条同样是越权接管账号
   （**重置 admin 密码 = 直接登录成 admin**，比授予角色更直接）

另外 `assign_role_menus`（`system:menu:grant`）本身就是"把权限码授予角色"的元能力，
若不设包含关系，持有它的角色可以**给自己授权全部权限码**，同样必须纳入判定。

## 起始 git 状态

- 分支 `v0.5.0`，工作区干净，HEAD = `88ca4055 docs(handoff): 记录 PR-2 commit 号`
- 之前 4 个 commit：`cc582550` / `1d1e0887` / `aa532547` / `9fb5e359`
- 仍未合并、未 tag、未发 Release

## PR-3 完成记录（2026-10-02）

### 落地范围

10 个文件，全部未提交：

| 文件 | 改动 |
|---|---|
| `src/middleware/permission.rs` | 核心：`first_uncovered` / `ensure_covers` / `codes_of_roles` / `ensure_can_grant_roles`；类型化守卫改为携带 `PermissionGuard`（`perm.guard()`）；+5 单测 |
| `src/middleware/auth.rs` | 删除 `require_role` |
| `src/router/mod.rs` | 删除 5 处 `require_role("admin")` |
| `src/controller/user.rs` | 7 条写路径接入授权下界 |
| `src/controller/role.rs` | 接入授权下界；`AssignRoleRequest.user_id` 改为 `Option<Uuid>` |
| `src/controller/menu.rs` | `update_menu` 严格守卫；`assign_role_menus` 只拦自授 |
| `src/repository/menu.rs` | `find_permission_codes_by_menu_ids` |
| `src/repository/role.rs` | `find_name_by_id` |
| `tests/api_integration.rs` | +10 集成测试、+3 契约测试、1 旧测试注释修正 |
| `docs/AI_HANDOFF.md` | 本文件 |

### 三个容易想错、已定案的点

1. **`assign_role_menus` 只拦"自授"**，不拦授予别人。禁掉"授予未持有的码给别的角色"
   会让 admin 无法分发新建的码（权限码即数据的核心工作流），且 PR-1 的
   `role_menu_query_keeps_grants_whose_ancestors_are_not_authorized` 会挂。
   间接路径仍闭合：先授给别人、之后该角色被授给自己时，由
   `ensure_can_grant_roles` 的包含关系拦住。
2. **`update_menu` 保留严格守卫**（改写 permission 必须持有目标码），
   因为 `menus.permission` 本身就是权限码，改写等于让"角色→菜单→码"这条链当场生效。
3. **`require_role` 整个函数删掉**，不留死代码——留着会被下一个人接回去。

### 测试与验证

- `cargo test --lib`：**49 passed**（44 → 49）
- `cargo test --test api_integration -- --include-ignored --test-threads=1`：**44 passed**（34 → 44）
- `cargo fmt --check` clean；`cargo clippy --all-targets -- -D warnings` clean
- `frontend` vitest：**81 passed**
- **缺陷注入 3 处**（摘掉守卫 → 用例如期失败 → 还原 → 全绿）：
  `resetting_a_stronger_account_password_is_denied`、
  `menu_grant_cannot_self_escalate`、
  `rewriting_a_granted_menu_permission_to_an_unheld_code_is_denied`
- 第三个用例改用**一次性临时码** `tmp:priv:xxxxxx`（admin 造一个不授予任何角色的按钮）。
  原写法拿真实码 `system:user:delete` 做两步攻击，会顺手把 admin 的该码摘掉，
  复原时又被守卫拦住（403）→ 共享库留一个洞，症状在别的用例上炸。
  守卫只比对**码的集合关系**、与码值无关，拦临时码 == 拦真实码。

### ⚠️ 共享测试库操作红线（本轮实际踩到）

清理测试库时我写了 `DELETE FROM menus WHERE permission IS NULL OR permission=''`，
**把 16 个种子目录菜单一起删了**（目录菜单的 permission 本来就是 NULL），
`menus` 表被清空、admin 码数归 0。

- 恢复方式：直接跑一次测试即可。`RbacService::init_defaults` 见到
  `roles` 非空但 `menus` 为空时仍会执行 `seed_navigation` →
  `seed_menus_if_empty`（menus 空则重灌）+ `seed_permission_codes`（**无条件**补齐 29 个码）。
  恢复后：42 菜单 / admin 28 码 / 28 按钮。
- **红线**：只按 `type='button' AND (permission IS NULL OR permission='')` 清理，
  绝不带 `type<>'button'`；或干脆让用例自己建、自己删。

### 运行测试的固定姿势

- 必须 `--test-threads=1`：共享库，并行会互相踩。
- **不要**用 `cargo test -- --include-ignored`：会把 doctest 一起强制跑，产生假失败。
  分开跑：`cargo test --lib` 和 `cargo test --test api_integration -- --include-ignored --test-threads=1`。
- 依赖环境：`eval "$(./scripts/test_env.sh env)"`（PG 55432 / `axum_api_test` / Redis 56379）。

### 遗留（未修，均已确认非本 PR 阻塞）

1. `update_menu` 把某按钮的 permission 清空后，该码**只能靠新建按钮恢复**——
   守卫不允许把别的菜单改指成这个未持有的码。属于"授权下界"的固有代价，
   管理员在 UI 上"清空权限码"后想反悔会比较绕。是否放开需产品决策。
2. 前端未改动：路由已按权限码/菜单动态注册，无角色闸门，逻辑上不受拆闸门影响
   （`frontend` 81 测试全绿佐证）。但**未做浏览器端人工回归**。
3. 缺陷注入时守卫被摘掉，测试真的把 admin 口令改成了 `hijacked123`
   （证明提权真实有效）。已用临时测试 `hash_password("admin123")` 直接 UPDATE 修复并删除该临时测试。

### 下一步

未 commit、未合并、未 tag、未发 Release。用户此前意图是走完 v0.3.0 同流程
（合并 → tag → Release），需先确认再执行。

### PR-3 commit 号

`4392f916` — feat(auth): 撤掉 require_role 角色闸门，改用权限码包含关系作为授权下界

## 升级路径验证（v0.4.0 → v0.5.0，真实二进制 + 真实存量库）

集成测试只覆盖空库启动，**不覆盖存量库升级**——这是真实用户最可能走的路径。
沿用 v0.3→v0.4 的做法：不用测试库，单独开一个库、两个真二进制。

1. `git worktree add /tmp/axum-v040 v0.4.0` → 编译 → 启动 → 产出**真实 v0.4.0 库**
   （迁移 001–007、42 菜单、28 button 行、28 个 permission 值）
2. 直接写库造一个非 admin 角色 `auditor`，只授予 `system:user:list`，建用户 `auditor1`
   （v0.4.0 无角色写接口，只能写库）
3. 停掉 v0.4.0，**用 v0.5.0 二进制启动同一个库**

### 结果

| 场景 | v0.4.0 | v0.5.0 |
|---|---|---|
| 迁移 | 001–007 | 001–008，`008 normalize role names` 正常应用 |
| 菜单 / 权限码 / 侧栏 | 42 / 28 / 14 | 42 / 28 / 14，**逐项未变** |
| `auditor`（持 `system:user:list`）`GET /api/admin/users` | **403**「需要 admin 角色权限」 | **200** ← 闸门确实拆掉了 |
| `auditor` `GET /api/admin/roles` | 403 | 403（缺 `system:role:list`，权限码守卫仍在） |
| admin 全部管理接口 | 200 | 200，**无 403 回归** |
| 存量自定义角色 `auditor` | 存在 | 迁移 008 后仍在，可正常登录 |

v0.4.0 侧的日志正好留下闸门开火的证据：
`require_role(admin): 用户 ... 角色 ["auditor"] 权限不足`。

### 提权路径闭合验证（本次最关键的一条）

把 `system:user:create` 也授予 `auditor`——它现在能过接口级权限码守卫，
但码集远小于 admin，正是"创建用户即等于授予管理员"这条路径的复现条件：

```
POST /api/admin/users  role=admin
  -> 403 "创建用户并赋予角色「admin」需要「system:dict:create」，
          而你未持有该权限码；只能授予自己已持有的权限"
POST /api/admin/users  role=user   -> 200        （无权限码的角色谁都能授予）
SELECT count(*) FROM users WHERE username='evil1' -> 0   （没留下半成品）
```

也就是说：v0.4.0 里这条路径是靠角色闸门**顺手**挡住的，闸门一撤就暴露；
本版由授权下界**显式**挡住，且 403 文案点名了缺哪个码。

### 复用要点

- 升级验证的库要**单独开一个**（本轮用 `axum_api_upgrade`），
  否则会污染 `axum_api_test`，症状会记到别人头上。
- 后台起服务必须用 **PTY 会话**（`exec_command` + `tty: true` + 长驻 session_id）。
  上一轮用 `nohup ... &` 起的进程会随 exec 的 shell 退出被回收，
  表现为 curl 返回 000、server 日志停在启动那一行——不是服务有问题，是进程没了。
- 同一端口只跑一个实例：v0.4.0 用 8081、v0.5.0 用 8082，便于对照。

## ✅ v0.5.0 已发布（2026-10-02）

- PR：https://github.com/ZhongGheart/axum-api/pull/4 （CI 三个 job 全绿后合并）
- 合并方式：merge commit `8768cd7c`（两个 parent，8 个提交历史全保留，未 squash）
- tag：`v0.5.0`（annotated，指向 merge commit `8768cd7c`）
- Release：https://github.com/ZhongGheart/axum-api/releases/tag/v0.5.0
- 已删除已合并分支 `v0.5.0`（本地 + 远程），master CI 复核全绿（run 36895031336）

### 发版流程里新增的一个坑（比 v0.4.0 多一条）

**本地分支与 tag 同名会让 ref 变得 ambiguous。** 打完 tag 后
`git rev-parse v0.5.0` 解析到的是**分支** tip（`a82a061c`）而不是 tag
（`8768cd7c`），并打印 `warning: refname 'v0.5.0' is ambiguous`。
tag 本身是对的（`refs/tags/v0.5.0^{commit}` 指向 merge commit），但任何走短名的
命令都会拿到错的 commit。

- 校验 tag 必须写全：`git rev-parse refs/tags/v0.5.0^{commit}`
- 推 tag 同理：`git push origin refs/tags/v0.5.0`（别写 `git push origin v0.5.0`）
- v0.4.0 记的 `dst refspec matches more than one` 是同一个根因的另一面
- 最省事的解法：tag 之前先 `git checkout master` 并删掉本地 `v0.5.0` 分支，
  名字就不会撞了。本轮是先 tag 后删分支，验证时必须用全 refspec。

其余照抄 v0.4.0 即可（`gh` 版本差异见上文）：`gh pr merge` 用 `-t/-b` 而非
`--title/--body`；`gh release create` 用 `--notes-file` + `--verify-tag`。

## 下一版候选方向（未开工，供取舍）

按"解除限制"的价值排序，不含推测性重构：

1. **多角色用户的前端表达**（当前唯一的语义不一致）：用户表单是整体替换、
   `POST /users/:id/roles` 是追加语义，多角色用户经表单保存会丢角色。
   这是本版遗留里唯一会导致**静默丢授权**的，其余都是能力缺口
2. **权限码清空后的恢复路径**：现在只能新建按钮才能拿回一个被清空的码，
   管理员在界面上"误清空 → 想反悔"没有出路。加一个"用已持有的码重建按钮"
   的入口，或让清空操作可撤销
3. **补前端入口**：`system:monitor:export`、`system:test:access` 后端已有码但前端无入口
4. **审计日志保留策略** + **接口耗时跨副本聚合**：都属运维债，
   接口耗时现在重启就丢
5. **浏览器端人工回归**：本版前端逻辑未改（路由按权限码动态注册），
   81 个前端测试 + 真实二进制升级验证已覆盖，但**没有做真人浏览器回归**。
   下一版动前端时建议补上

---

# v0.6.0 — 用户角色的多角色表达

## 当前目标

按 v0.5.0 发布后定的后续计划**顺序**执行。第 1 项：
修掉"多角色用户经用户表单保存会静默丢角色"——本版遗留里唯一会导致
**静默丢授权**的问题。

## 缺陷（已确认，两个面）

数据模型是多角色的（`user_roles` 表、`UserInfo.roles`、`POST /users/:id/roles`
追加语义），但用户表单是单角色的，两边对不上：

1. **保存即丢角色**：`currentRoleName(user)` 返回 `roles[0]`，
   表单 `n-select` 是单选，回填只显示第一个角色；提交 `role: "admin"`，
   后端 `replace_user_roles(id, &["admin"])` **整体替换**——
   用户原有的第二个角色被无声删除，且**没有任何提示**
2. **列表也只显示一个**：`columns` 里角色列同样用 `currentRoleName(r)`，
   渲染单个 `NTag`。多角色用户在列表里看起来就是单角色，
   管理员根本看不出这个用户其实有额外权限

## 计划（PR-1）

### 后端

- `UserManageRequest`：`role: String` → `role: Option<String>`（兼容别名，标 deprecated）
  \+ 新增 `roles: Option<Vec<String>>`（**权威字段**）。
  两者至少给一个；都给时以 `roles` 为准
- 新增 `resolve_roles(state, &[String]) -> Result<Vec<String>>` 取代 `resolve_role`：
  逐个归一化 → 去重 → 校验角色真实存在 → **要求非空**
  （零角色的用户是"建得出、没人能用"的死数据，与 PR-1 修的半成品用户同类）
- `create_user` / `update_user` 改用 `resolve_roles`，
  授权下界守卫直接传整个 `new_roles` 切片（`ensure_can_grant_roles`
  本来就是按切片比码集，无需改动语义）
- `ensure_not_last_admin` / `same_role_set` 本来就吃切片，天然支持多角色

### 前端

- 表单角色项改 `n-select multiple`，`formData.roles: string[]`
- `utils/role.ts`：`currentRoleName` → `currentRoleNames`（返回数组，
  兼容只回 `role` 的老响应）；`buildRoleSelectOptions` 的 `current`
  收数组，把**所有**不在列表里的当前角色补进去；
  `pickDefaultRole` → `pickDefaultRoles`
- 列表角色列渲染**全部**角色为多个 tag（admin 用 warning 色，其余 info）
- `api/user.ts` 的 `create`/`update` 请求体发 `roles: string[]`

### 测试

- 后端：多角色建号落库；更新时保留未在表单里体现的角色（**回归核心**）；
  空 `roles` 拒绝；未知角色拒绝且不留半成品；重复角色去重；
  授权下界对多个角色逐个生效（持 `user:create` 建多角色含 admin → 403）
- 前端：`currentRoleNames` 与选项补全的契约测试
- 兼容：`role` 单数字段仍可用（既有客户端与既有测试不改）

## 起始 git 状态

- 分支 `v0.6.0`（从 master 切出），工作区干净
- HEAD = `cf8cbdde docs(handoff): 记录 v0.5.0 发布结果与分支/tag 同名的 ref 歧义坑`
- 上一版：v0.5.0 已发布（tag `v0.5.0` → merge commit `8768cd7c`）

## 后续（本次不做，按序排在后面）

2. 权限码清空后的恢复路径
3. 补前端入口（`system:monitor:export`、`system:test:access`）
4. 运维债：审计日志保留策略、接口耗时跨副本聚合

---

## PR-1 完成记录（实现 + 验证全绿，待发布）

### 落地决策

1. **修的是"静默丢授权"**：v0.5.0 遗留里唯一会导致静默数据丢失的问题。
 数据模型一直多角色，但写入口是单数 + 整体替换语义
2. **两个面都修了**：表单（多选框）+ 列表列（此前只渲染 `roles[0]`，
 多角色用户看起来就是单角色）
3. **`role` 保留为兼容别名**（标 `#[deprecated]`，配 `#[allow(deprecated)]` 读取），
 `roles` 为权威字段；都给时 `roles` 优先。既有客户端与既有测试零改动——
 `the_legacy_single_role_field_still_assigns_one_role` 未作任何改动即通过，
 这是向后兼容的直接证据
4. **`resolve_roles` 要求非空**：零角色用户是"建得出、没人能用"的死数据。
 去重保序，**先全校验再写**（原子）
5. `ensure_can_grant_roles` / `ensure_not_last_admin` / `same_role_set`
 本来就吃切片，**无需改动语义**——多角色不会放宽授权

### 测试数字

| 项目 | v0.5.0 | v0.6.0 |
|------|--------|---------|
| `cargo test --lib` | 49 | 49 |
| `cargo test --test api_integration`（`--include-ignored --test-threads=1`） | 44 | **51**（+7） |
| 前端单测 | 81 | **84**（+3） |
| `cargo fmt --check` | clean | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean | clean |
| 前端 typecheck / lint | clean / 0 error | clean / 0 error（1 个 `env.d.ts` 历史 warning） |

新增 7 条集成测试：`creating_a_user_with_several_roles_assigns_all_of_them`、
`updating_a_user_round_trips_every_role`、`an_empty_role_list_is_rejected_and_changes_nothing`、
`one_unknown_role_rejects_the_whole_list`、`duplicate_roles_are_collapsed`、
`the_legacy_single_role_field_still_assigns_one_role`、
`granting_several_roles_at_once_still_hits_the_superset_rule`

### 缺陷注入验证（新测试确实能抓到该缺陷）

把 `requested_roles` 改成 `vec![roles.first()...]`（模拟旧单角色行为）后：

- `creating_a_user_with_several_roles_assigns_all_of_them` **FAILED**，
 输出 `left: ["role_a_…"] right: ["role_a_…", "role_b_…"]`——真实只落 1 个角色
- `updating_a_user_round_trips_every_role` **FAILED**，`left: 1 right: 3`
- `duplicate_roles_are_collapsed` 仍通过（正确：3 个相同折叠成 1，结果相同）

已还原（`roles.clone()`，`src/controller/user.rs:115`）。

### 测试库清理方式（**注意：不要做手术式 DELETE**）

注入失败那轮留下了 `multi_role_user_*` 用户和 `role_a_*`/`role_b_*`/`rt_*`/`dup_*` 角色。
上一轮曾因 `DELETE FROM menus WHERE type<>'button'` 误删 16 个种子目录菜单。

**本轮改为整库重建**（`dropdb` + `createdb` + `redis-cli flushall`），
然后在**完全空库（0 张表）**上重跑全量集成测试：51 全绿。
这样既清干净，又顺带验证了空库迁移 + 种子数据引导。
`scripts/test_env.sh stop` 本来就是 `rm -rf $PGDATA`——这个测试库是完全一次性的。

重建后种子数据核验：

```
menu_total=42   dir_menus=14   button_menus=28
buttons_no_permission=0        tmp_leftover=0
roles=15        admin_role=1   distinct_admin_users=1
admin_granted_perm_codes=28
```

红线（若确实需要手术式清理菜单时）：只按
`type='button' AND (permission IS NULL OR permission='')` 清理，
**绝不带 `type<>'button'`**。

### 本次收尾的文档/版本改动

- `CHANGELOG.md`：新增 `[0.6.0]` 段（修复 / 变更 / 破坏性变更 / 测试）
- `README.md`：版本导语补 v0.6.0 多角色；能力清单"用户管理"行补**多角色分配**
- `Cargo.toml` + `frontend/package.json`：`0.5.0` → `0.6.0`

### 尚未完成

1. 提交 + push 分支 → 开 PR → CI 三 job（rust / frontend / docker）全绿
2. merge commit → annotated tag `v0.6.0` → `gh release create --notes-file --verify-tag`
3. 删分支（用全 refspec `:refs/heads/v0.6.0`）
4. 可选但推荐：升级路径实测（v0.5.0 → v0.6.0）。v0.6.0 **无新迁移**，
 成本低；v0.5.0 的发布流程里做了，保持一致

---

## 升级路径验证（v0.5.0 → v0.6.0，真实二进制 + 真实存量库）

方法沿用 v0.4→v0.5：`git worktree add /tmp/axum-v050 v0.5.0` → 编译 release →
单独开库 `axum_api_upgrade` → v0.5.0 造数据 → 停掉 → **v0.6.0 二进制启动同一个库**。
全程用 PTY 会话起服务（`nohup ... &` 会被 exec shell 退出回收）。

### v0.5.0 侧：先复现缺陷（本版要修的正是它）

v0.5.0 有追加语义的 `POST /api/admin/users/{user_id}/roles`，
所以**真实 v0.5.0 库里确实存在多角色用户**，不是理论问题。
用该端点把 `multi_v050` 造成 `user` + `auditor` 两个角色，再用表单保存：

    PUT /api/admin/users/{id}  {"role":"user"}    ← 模拟前端只回填 roles[0]
      -> HTTP 200  {"message":"success"}
    SELECT r.name FROM user_roles ...            -> 只剩 user

`auditor` 被静默删除，**接口返回成功**。这是升级前真实存在的状态。

### 数据快照对照（升级前后逐项比对）

| 项 | v0.5.0 | v0.6.0 启动后 |
|---|---|---|
| 迁移版本 | 008 | 008（**v0.6.0 无新迁移**） |
| 菜单 / 按钮 / 权限码 | 42 / 28 / 28 | 42 / 28 / 28，**逐项未变** |
| 角色 / 用户 | 3 / 3 | 3 / 3，**未变** |
| 存量多角色用户 `multi_ok` | `["user","auditor"]` | `["user","auditor"]`，**完整保留** |

### v0.6.0 侧行为验证（同一存量库）

| 场景 | 结果 |
|---|---|
| 新写法 `roles:["user","auditor"]` 保存 | 200，两个角色**都在** |
| 给受损的 `multi_v050` 加回 `auditor` | 200 → `["user","auditor"]`（v0.5.0 做不到） |
| 三角色 `["user","auditor","viewer"]` | 200 → 三个都在 |
| 旧客户端只发 `role:"user"` | 200 → `["user"]`，**整体替换，与 v0.5.0 完全一致** |
| 空 `roles:[]` | 400「至少需要指定一个角色：没有角色的用户登录后没有任何权限」 |
| `role` 与 `roles` 都给 | `roles` 优先 → 三个角色 |
| 重复角色 `["user","user","auditor","user"]` | 折叠为 `["user","auditor"]`，库内 2 行（无重复行） |
| 未知角色 `["user","nonexistent_zzz"]` | 400 点名该角色；角色行数 before=2 after=2，**原子拒绝** |
| admin 全部管理接口（users/roles/menus/audit-logs/monitor） | 全 200，**无 403 回归** |
| 多角色并集越界 `roles:["user","admin"]` | 403，文案点名缺 `system:dict:create`；`evil2` 建号数 **0** |
| 唯一 admin（持 `admin`+`user`）改成只留 `user` | 400「不能移除最后一名管理员的 admin 角色」 |
| 删除唯一 admin | 400「不能删除当前登录账号」 |

授权下界对**多角色并集**依然成立：持 `system:user:create`+`system:user:list`
的角色能建 `user`（无权限码的角色），但建 `user`+`admin` 被拒，
且 403 文案点名缺哪个码。

### 复用要点

- 升级验证的库要**单独开一个**（本轮 `axum_api_upgrade`），验证完 `dropdb`，
  否则会污染 `axum_api_test`，症状记到别人头上
- 同一端口串行跑两个版本：v0.5.0 与 v0.6.0 都用 8083，停干净再起下一个
  （`curl` 返回 000 才算真停）
- `assign_role_menus` 的 `menu_ids` 必须是**带引号的 UUID 数组**；
  用 shell 拼字符串时漏了逗号会报 `invalid type: integer`，容易被误读成授权失败
- zsh 里**不要用 `UID` 作变量名**（是只读的当前用户 id），本轮踩过

---

## ✅ v0.6.0 已发布（2026-10-02）

- PR：https://github.com/ZhongGheart/axum-api/pull/5
  （CI 三 job：rust / frontend / docker 全绿后合并）
- 合并方式：merge commit `4d036487`（两个 parent，历史全保留，未 squash）
- tag：`v0.6.0`（annotated，指向 merge commit `4d036487`，已用
  `git rev-parse refs/tags/v0.6.0^{commit}` 校验）
- Release：https://github.com/ZhongGheart/axum-api/releases/tag/v0.6.0
- 已删除已合并分支 `v0.6.0`（本地 `git branch -D` + 远程
  `git push origin --delete refs/heads/v0.6.0`）
- master CI 复核：run 36939961715

### 本轮验证过 tag/branch 同名的坑没再踩

沿用 v0.5.0 的解法：**打 tag 之前先 `git checkout master` 并删掉本地
`v0.6.0` 分支**，这样 `v0.6.0` 这个短名只解析到 tag。
本轮 `git rev-parse refs/tags/v0.6.0^{commit}` 一次就对上了 merge commit，
没有出现 ambiguous 警告。

### 环境清理

- 升级验证库 `axum_api_upgrade` 已 `dropdb`
- worktree `/tmp/axum-v050` 已 `git worktree remove --force`
- `axum_api_test` 保留在本轮整库重建后的干净状态（空库跑完 51 条后的状态）

## 后续版本计划（按序，v0.6.0 已完成第 1 项）

1. ~~多角色用户（PR-1）~~ ✅ **v0.6.0 已发布**
2. **权限码清空后的恢复路径**：`update_menu` 清空某按钮的 `permission`
 后，该码只能靠新建按钮恢复——授权下界不允许把别的菜单改指成未持有的码。
 需要给出"权限码丢了怎么找回来"的可操作路径
3. **补前端入口**：`system:monitor:export`、`system:test:access`
 两个权限码后端已支持、种子已下发，但前端没有任何入口能用到它们
4. **运维债**：审计日志保留策略；接口耗时跨副本聚合
 （现在是进程内统计，重启丢失，多副本下不准）

---

# v0.7.0 — 权限码清空后的恢复路径（计划第 2 项）

## 当前目标

按后续版本计划**顺序**执行。第 1 项（多角色用户）已在 v0.6.0 发布完成。
本版做第 2 项：**`update_menu` 清空某按钮 permission 后，该码怎么找回来。**

## 缺陷（已确认）

`update_menu` 的守卫是**单向**的：

```rust
if let Some(new_permission) = req.permission.as_deref().filter(|p| !p.is_empty()) {
    perm.guard().ensure_covers(&required, "把菜单的权限码改为该值")?;
}
```

`.filter(|p| !p.is_empty())` 让**清空**绕过守卫。于是：

1. 清空某按钮的 `permission` → 该码在全系统消失，**没有任何角色再持有它**
2. 想写回去 → 守卫要求持有该码 → **没人持有** → 403

**死锁**：唯一出路是 `create_menu` 造一个新按钮（该端点无守卫），
但那会留下一个位置/父节点/名称都不对的孤儿菜单，原按钮的 `role_menus`
授权还在、却不再对应任何码，管理员看不出原来那个码是什么。

附带确认：`create_menu` 的 `_perm: PermMenuCreate` **完全没用到**，
即"定义一个权限码"这件事没有任何授权下界。

## 方案

### 1. 清空也要过守卫，但只在"真的撤销了别人的权限"时

- 该按钮**已被授予至少一个角色** → 清空就是从那些角色手里收回码
  → 要求调用者**持有该码**（与"设置新码必须持有"对称）
- 该按钮**未授予任何角色** → 清空不改变任何人的权限
  → 放行（保持既有测试 `rewriting_a_granted_menu_permission_to_an_unheld_code_is_denied`
  的第一步仍然成立，该测试**不得改动**）

这样补上了真实缺口：此前持 `system:menu:update` 的角色可以
把**别的角色**已持有按钮的码清掉，绕过 `system:menu:grant`。

### 2. 清空时留痕 + 提供"撤销我自己的误操作"

迁移 `009` 给 `menus` 加两列：

- `prev_permission` — 清空前的码
- `prev_permission_cleared_by` — 谁清的

新增 `POST /api/admin/menus/:id/restore-permission`：

- **只有清空者本人可调**（按 user id 比对）
- 恢复 `prev_permission`，**不要求当前持有该码**

### 为什么恢复不构成提权（这是本方案成立的关键）

清空已授予角色的按钮要求"持有该码"，所以能清空 ⟹ 当时持有。
恢复只是把状态还原到清空之前，**净零**。
若菜单未授予任何角色则清空本就放行，恢复也只是给"无人"一个码，同样净零。

而"把菜单授予别的角色"这条路仍要过 `system:menu:grant` +
`assign_role_menus` 既有判定，因此恢复不构成新的越权原语。

### 3. 前端

菜单管理页在该按钮存在 `prev_permission` 时显示"恢复权限码"操作。

## ~~本版不做（留给下一项）~~ —— 该判断已被实测推翻，勿照此施工

原文（v0.7.0 写下）："`create_menu` 无授权下界：持 `system:menu:create` 的角色
可以声明一个后端认识的码（如 `system:user:delete`）再分发给别的角色。
候选规则：只允许声明"当前没有任何菜单在用"的码。"

**v0.8.0 用探针实测后推翻**：那个洞早被迁移 `007` 的部分唯一索引
`idx_menus_permission_unique` 在数据库层堵住了，本节提议的规则
索引已经免费提供，无需再实现。当时真正没堵的是 `delete_menu`，
已在 v0.8.0 第 1 项修掉。

**留在这里而不是删掉，是为了防止后人照着这段错误前提去写代码或开新任务。**

## 起始 git 状态

- 分支 `v0.7.0`（从 master 切出）
- HEAD = `d91dbbf0 docs(handoff): 记录 v0.6.0 发布结果与后续版本计划`
- 上一版：v0.6.0 已发布（tag `v0.6.0` → merge commit `4d036487`）

## PR-1 完成记录（实现 + 验证全绿，**尚未推送**）

按用户指令：**本地跑门禁 + 提交，不推送**，等后续版本计划全部做完再统一推送。

### 落地内容

- 迁移 `009`：`menus` 加 `prev_permission` / `prev_permission_cleared_by`
- `update_menu`：清空**已授权**按钮的码也要持有该码；未授权的照旧放行
- `restore_menu_permission`：新端点 `POST /api/admin/menus/:id/restore-permission`，仅清空者本人
- repo：`restore_permission` / `is_granted_to_any_role`；`MENU_COLUMNS` 常量收口 4 处列清单
- `MenuNode`：新增 `restorable_permission`；**刻意不暴露** `cleared_by`
- 前端：菜单树行显示"码已清空，可恢复 X"标签 + 恢复按钮

### 真实浏览器回归 13/13（`/tmp/axum-e2e/delguard.mjs`）

集成测试证明不了"守卫没有误伤正常管理流程"，所以补了真实 Chrome 回归：
无码页面菜单可正常新建+删除（放行）、被他人依赖的码删除被拒 403 且码没被剥掉、
撤销授权后删除放行（恢复路径真实可用）、菜单页正常渲染、无 console error。

**这次回归顺带挖出 `menus_type_check` 也报 500**（见上方"计划外补的一项"）——
我的脚本传了 `type: "page"` 拿到 500 才注意到。教训：
写浏览器回归脚本时用错枚举值，恰好暴露了产品缺陷。

### 门禁结果（全绿）

- `cargo test --lib`：49
- 集成测试：51 → **56**（+5）
- 前端单测：84
- `cargo fmt --check` / `clippy -D warnings`：clean
- 前端 typecheck / lint / build：clean / 0 error

新增 5 条：`clearing_a_permission_code_can_be_restored_by_the_clearing_user`、
`clearing_a_code_others_rely_on_requires_holding_it`、
`clearing_a_code_no_role_relies_on_is_allowed`、
`only_the_clearing_user_can_restore_a_permission_code`、
`the_menu_tree_reports_a_restorable_permission_code`

空库重建后跑全量：56 全绿，`max_migration=9`，
种子核验 `menu_total=42`（14 目录 + 28 按钮）/ `perm_codes=28` /
`stale_prev_slot=0` / `admin_role=1` / `distinct_admin_users=1` / `tmp_leftover=0`。

### 缺陷注入验证（两条都真实失败）

1. 移除 `update_menu` 的清空守卫 →
   `clearing_a_code_others_rely_on_requires_holding_it` **FAILED**，
   实得 200（别人依赖的码被静默清掉），期望 403
2. 移除 `restore_menu_permission` 的归属校验 →
   `only_the_clearing_user_can_restore_a_permission_code` **FAILED**，
   实得 200（非清空者成功接管），期望 400

两处均已还原（`src/controller/menu.rs:181` / `:235`）。

### 写测试时踩到的两个坑（都是真·授权下界，不是测试写错）

1. **admin 不持有自建的码**。种子的"只授权新建行"策略刻意不把新码塞给
   admin（否则管理员在菜单页撤销的授权会被下次启动悄悄恢复）。
   所以"admin 造一个码再清空它"必然 403——admin 恰恰是不持有它的人。
2. **建号顺序被授权下界卡死**。`ensure_can_grant_roles` 要求
   "建号时赋予的角色，其码集 ⊆ 你的码集"。先授权再建号会失败，
   因为 admin 不持有那个新码。必须**先建空角色与用户、再授权按钮**。

夹具 `granted_temp_button` 因此自带两条自检断言（持有者必须真的经
"角色→菜单"拿到该码、且必须持有 `system:menu:update`），
否则夹具静默失效、测试会假绿。

## PR-2 完成记录（计划第 3 项：补前端入口 + 真实浏览器回归，**尚未推送**）

v0.5.0 的 handoff 里留了一句债：「没做浏览器回归，下一版动前端时补上」。
本轮动了前端，因此补上——用**真实 Google Chrome 154** 跑完整登录→点击→下载链路。

### 落地内容

- `constants/permission.ts`：补 `PERM.MONITOR_EXPORT`、`PERM.TEST_ACCESS`
- `api/monitor.ts`：补 `exportSystem()`（blob 响应）
- 监控页 `monitor/system/index.vue`：页头加「导出 Excel」按钮 + `handleExport`
- 后端能力页 `demo/backend.vue`：加第 4 张卡「能力探测」+ `probeAccess`
- `__tests__/permissionCodes.spec.ts`：`BACKEND_ONLY_CODES` 清空为 `{}`

`BACKEND_ONLY_CODES` 清空是这个测试的**意图兑现**：它一直是用来显式登记
「后端有码但前端没入口」的豁免清单，清空意味着后端 28 个权限码
**全部**在前端有对应入口，契约测试从此双向全覆盖。

### 浏览器回归怎么做（环境无 Playwright）

机器上没有 Playwright/puppeteer，也不想为此拉依赖。
Node 22 自带全局 `WebSocket`，于是直接用 **CDP** 驱动**真实的 Google Chrome**
（`--headless=new --remote-debugging-port=9222`），脚本在
`/tmp/axum-e2e/`（一次性，不入库）。三个要点：

1. **每次跑新建 browser context**：否则上次的 token 还在，
   `/login` 会直接跳首页，脚本卡在"找不到登录框"
2. **Chrome 必须起在 PTY 里**：普通后台 `&` 会随 shell 退出被回收，
   表现为跑到一半 `ECONNREFUSED 9222`
3. **`localStorage` 是 base64(encodeURIComponent(JSON))**：
   `JSON.parse(localStorage.getItem('axum_token'))` 会报
   `Unexpected token 'J'`，必须先 `decodeURIComponent(atob(...))`
   再 `.value`

### 回归结果：13/13 通过

真实登录 admin → 监控页点「导出 Excel」→ 文件落盘且是合法 xlsx
（`magic=PK`、`xl/worksheets/sheet1.xml` 存在、6467 字节）
→ 后端能力页点「探测访问能力」→ 「管理员访问成功！用户: …, 角色: ["admin","user"]」
→ 全程无 4xx/5xx、无控制台错误。

截图在 `/tmp/axum-e2e/shots/`，已人工核对版式：
导出按钮在页头与「刷新」并排不重叠，第 4 张卡文案不溢出。

### 回归顺手挖出并修掉的一个真缺陷

截图里第 2 张卡（通用分页查询）显示「无数据」，而同一时刻
`GET /api/admin/users?page=1` 返回 `total=1, items=1`。

根因：`demo/backend.vue` 的 `fetchTest` **只挂在分页的 `@update:page` 上**，
从没在 `onMounted` 触发。进页面表格恒为空，
要点一次页码才出数据。已加 `onMounted(fetchTest)`（一行）。

这个 bug 与本次改动无关（是既有的），但它只有真人看截图才会发现——
自动化断言若只查"接口返回 200"就会漏过去。
回归脚本里已补成硬断言：表格行数必须等于 `items.length`。

## 待办

1. 计划第 4 项：运维债（审计日志保留策略、接口耗时跨副本聚合）
2. 全部完成后统一推送 `v0.7.0`（当前 `ad09d294` + 第 3 项两个 commit 均未推送）

# v0.7.0 计划第 4 项 — 运维债（审计日志保留、接口耗时跨副本聚合）

## 动手前确认的两件事（其中一件推翻了原计划）

### 1. 审计日志确实无界增长

`audit_logs` 每个已认证请求插一行，`middleware::audit_log` 无任何清理，
表结构也没有分区。`idx_audit_logs_created` 只加速查询，不限制增长。

### 2. 指标路径未归一化——这是原计划的**前置阻塞项**

`api_metrics_mw` 记的是 `req.uri().path()`，即**含真实 ID 的具体路径**。
实测（admin token，打两个不同的 UUID）：

```
总条目: 13 | 含 UUID 路径条目: 2
    GET /api/admin/users/11111111-1111-4111-8111-111111111111 calls= 1
    GET /api/admin/users/22222222-2222-4222-8222-222222222222 calls= 1
```

每个资源 ID 一条独立记录，于是：

- **基数无界**：HashMap 按 ID 无限增长
- **监控页失去意义**：`/api/admin/users/{id}` 每个只出现一次、count=1，
  管理员看不出这个接口真实的 QPS 与耗时分布

原先"进程内"只是让这个缺陷表现为内存涨；**一旦改成 Redis 聚合，
就变成每个 UUID 一个 Redis key**，从内存问题升级成共享 Redis 的内存问题。
因此路径归一化必须与聚合一起做，且**先做**。

## 方案

### 4a. 路径归一化（前置）

改用 axum 的 `MatchedPath` 扩展取**路由模板**（`/api/admin/users/{id}`），
未匹配路由（404）回退到原始 URI 路径——否则 404 流量会全部塌成一个键。

### 4b. 指标聚合落到 Redis

`MetricsCollector` 改为「本地增量缓冲 + 定时 flush 到 Redis」：

- 本地 `Mutex<HashMap>` 只做合并，不碰网络（请求路径上零 Redis 往返）
- 后台任务每 `METRICS_FLUSH_INTERVAL_SECONDS`（默认 5s）把增量 `HINCRBY` 进 Redis
- `snapshot()` 时把**尚未 flush 的本地增量**合并进 Redis 读数，
  免得页面最多等 5 秒才看到刚发生的调用
- Redis key `metrics:ep:{method} {path}`，HASH 存 `c/e/t/mx/mn`，TTL 兜底防泄漏
- **max/min 用 Lua `EVAL` 原子更新**：读改写会有竞态，
  两个副本同时上报时可能丢一次极值
- **flush 失败不清空本地缓冲**，留到下一轮重试。Redis 短暂故障不丢指标；
  缓冲超过上限则告警并丢弃，防止 Redis 长期不可用时无限涨
- `reset()` 改为「清本地缓冲 + 删 Redis 全部键」，
  即**跨副本重置**。此前只清本进程，别的副本照旧累加——这本身是个真 bug

优雅关闭时做最后一次 flush，损失上界为 0（崩溃则 ≤ 一个 flush 间隔）。

顺带修一个既有隐患：`min_duration_ms` 以 `0` 当"未设置"，
但亚毫秒请求的耗时**真的就是 0**，会被后续更大的值覆盖。改用 `Option<u64>`。

### 4c. 审计日志保留策略

新增 `AuditLogConfig`（`AUDIT_LOG_RETENTION_DAYS` 默认 90 天，
`AUDIT_LOG_CLEANUP_INTERVAL_SECONDS` 默认 3600，
`AUDIT_LOG_CLEANUP_BATCH_SIZE` 默认 10000）。设为 0 即关闭自动清理。

删除**分批**执行，每轮最多若干批、删空即止：

```sql
DELETE FROM audit_logs WHERE id IN (
  SELECT id FROM audit_logs WHERE created_at < $1 ORDER BY created_at LIMIT $2
);
```

一次性 `DELETE` 大量行会长时间持锁并膨胀 WAL；分批把锁持有时间切碎。
借 `idx_audit_logs_created` 反向扫描最旧的一批。

**删除必须留痕**：`tracing::info!` 记录截止时间点与删除行数。
审计数据被静默删除是不可接受的——出事后没人知道日志是什么时候没的。

## 后台任务归属

后台任务由 `main.rs` 启动，**不放进 `create_router`**：
测试反复构建应用，若在 `create_router` 里 spawn，
每个测试进程都会残留一批清理任务去打共享测试库。
进程生命周期归 `main` 管，应用构建只管组装。

## 起始 git 状态

- 分支 `v0.7.0`，HEAD = `0453e060`（计划第 3 项）
- 工作区干净；`ad09d294` / `0453e060` 均**未推送**

## 计划第 4 项完成记录（运维债，**尚未推送**）

### 落地内容

- `config`：`AuditLogConfig` + `MetricsConfig`（见下表环境变量）
- `middleware/api_metrics.rs`：**整体重写**为「本地增量缓冲 + 定时 flush 到 Redis」
- `service/audit_retention.rs`：新增审计日志保留后台任务
- `repository/audit_log.rs`：`delete_older_than`（分批）
- `router/mod.rs`：`create_router` 返回 `(Router, AppState)`，
  collector 改用 Redis 构造；后台任务**不在这里 spawn**
- `main.rs`：启动两个后台任务，优雅关闭时先停任务（触发最后一次 flush）

### 新增环境变量

| 变量 | 默认 | 含义 |
|---|---|---|
| `AUDIT_LOG_RETENTION_DAYS` | 90 | 审计日志保留天数，0 = 关闭自动清理 |
| `AUDIT_LOG_CLEANUP_INTERVAL_SECONDS` | 3600 | 清理间隔 |
| `AUDIT_LOG_CLEANUP_BATCH_SIZE` | 10000 | 单批删除行数 |
| `AUDIT_LOG_CLEANUP_MAX_BATCHES` | 20 | 单轮最多批数 |
| `METRICS_FLUSH_INTERVAL_SECONDS` | 5 | 指标 flush 间隔（崩溃最多丢这么久） |
| `METRICS_KEY_TTL_SECONDS` | 604800 | Redis 指标键 TTL（7 天） |
| `METRICS_MAX_BUFFERED_ENDPOINTS` | 10000 | 本地缓冲端点数上限（Redis 长期挂时的兜底） |

### 门禁结果（全绿）

- `cargo fmt --check` / `clippy -D warnings`：clean
- `cargo test --lib`：49 → **55**（+6）
- 集成测试：56 → **63**（+7）
- 前端未改动（本项纯后端），故未重跑前端门禁

新增 6 条单元测试（合并算术、0 值 min 语义、avg 不除零、拆键剥前缀）；
新增 7 条集成测试（路径模板归并、跨副本聚合、跨副本重置、未 flush 增量可见、
部分已落库仍为一行、审计只删过期、分批且受上限约束）。

### 缺陷注入验证（两条都真实失败）

1. 路径改回 `req.uri().path()` → 路径模板归并测试 FAILED：
 实得**两行、每行 call_count=1**（正是原缺陷），期望一行 call_count=2
2. `reset()` 改回只清本地缓冲 → 跨副本重置测试 FAILED：实得残留数据

### 注入过程中暴露的两个真 bug（都已修 + 补测试）

这是本次最值得记的部分——**注入验证不只是"确认测试有效"，
读失败输出时发现了实现本身的两个 bug**：

1. **`method` 变成 `metrics:ep:GET`**。`split_key` 忘了剥 Redis 键前缀。
   之前所有测试都只断言 `path`（剥不剥前缀 path 都对），所以一直没暴露；
   是失败输出里那行 `"method": "metrics:ep:GET"` 让我发现的。
   已修 + 补单测 + 在集成测试里显式断言 `method == "GET"`
2. **同一端点裂成两行**。`snapshot()` 合并时，Redis 键带前缀、本地缓冲键不带，
   两者塞进同一张 map 成为两条独立记录 → **重复计数**。
 已修（插入前统一 `normalize_key`）+ 补测试
   `one_endpoint_stays_one_row_when_it_is_partly_flushed_and_partly_pending`。
   这个测试也是先写出来才暴露的——没有它，这个 bug 会带着"看起来正常"的
   分行数据进生产

教训：**只断言部分字段的测试，会给未断言字段留出静默出错的空间**。
 拆键、前缀这类"看起来无所谓"的地方最容易漏。断言要覆盖到人眼会看的每一列。

### 真实进程验证（不止单测）

- 起真实二进制（`METRICS_FLUSH_INTERVAL_SECONDS=2`、`AUDIT_LOG_CLEANUP_INTERVAL_SECONDS=20`）：
 三个不同 UUID 的请求归并成一行 `calls=3`；Redis 键 TTL ≈ 604785s（7 天）；
 HASH 字段为 `c/e/t/mx/mn`
- 造 1200 条过期日志 → 20s 周期自动清空（分批 500），保留期内 108 条未受影响
- 保留任务 `info` 日志确认留痕：
 `已清理过期操作日志 截止时间=... 删除行数=9`
- 优雅关闭：请求后立即 SIGTERM，指标仍落库，日志
 `接口指标已全部写入 Redis`
- 真实 Chrome 打开接口监控页：按模板聚合、`method` 干净、`{user_id}` 一行

### 已知取舍（诚实记录，非缺陷）

- Redis 被 flush 或未持久化时指标会丢。对监控数据可接受，
 换来跨副本准确 + 重启不丢（优雅关闭不丢，崩溃最多丢一个 flush 间隔）
- 多副本各自跑清理任务，无选主。因删除分批、走同一条索引、幂等，
  只是徒增锁竞争、不会错删，故未引入分布式锁

## v0.7.0 当前状态

- 计划 4 项已全部完成
- 共 3 个 commit：

  | commit | 内容 |
  |---|---|
  | `ad09d294` | 计划第 2 项：权限码清空后的恢复路径 |
  | `0453e060` | 计划第 3 项：补齐两个权限码的前端入口 |
  | `5d8610d8` | 计划第 4 项：指标跨副本聚合 + 审计日志保留 |

- **已按用户指令统一推送**：`origin/v0.7.0`（用户原话："等全部工作完成或收到指令在统一推送"，
  四项做完即触发）。推送前每一项都先本地跑完质量门禁再提交
- CI 仅在 `master` push 与 PR 上触发，**单纯推分支不会起 CI**；
  当前 `v0.7.0` 分支上**没有 CI 运行记录**，需开 PR 才会跑。
  本地门禁已全绿（见各项完成记录），但**远端 CI 尚未验证**
- 下一步：已发布 v0.7.0（2026-10-02）。分支 `v0.7.0` 尚未删除，待用户确认后再删。
  合并与打 tag 有后果，已按用户指令执行完毕。

## PR #6 已开 + 按 CI 原始命令复核（v0.7.0）

- 开 PR：<https://github.com/ZhongGheart/axum-api/pull/6>（v0.7.0 → master）
- **CI 由此首次真正跑起来**。此前 v0.7.0 分支上零 CI 记录

### 关键补测：本地门禁此前并不等于 CI 门禁

CI 的 clippy 与 test 都带 `--locked --all-targets --all-features`。
上一轮本地跑的命令比这窄 —— 不含集成测试 target、也不含全 feature 组合。
故按 CI 原始命令重跑了一遍：

| 命令（CI 原始） | 结果 |
|---|---|
| cargo fmt --all --check | clean |
| cargo clippy --locked --all-targets --all-features -- -D warnings | clean，无新警告 |
| cargo test --locked --all-targets --all-features | 55 passed |
| cargo test --locked --test api_integration -- --ignored --test-threads=1 | 60 passed |
| pnpm lint（exit code 实测） | 0，脚本无 --max-warnings，历史 warning 不会红 CI |
| pnpm typecheck / pnpm test / pnpm build | clean / 84 passed / exit 0 |

集成测试口径澄清：CI 用 `--ignored`，只跑被标记的那 60 条。
另 3 条普通测试被 filtered out。两者相加正好 63，与前文记录的 63 一致。
所以不是少跑了 3 条，而是 63 条里有 60 条带 ignore 标记。

### 本机无法覆盖的门禁

- docker job 只能靠 CI。本机没有可用 Docker daemon，
 镜像构建与 compose 配置校验无法本地复现

### CI 结果（run 36946178817，PR #6）

三个 job 全绿：

| job | 结果 | 耗时 |
|---|---|---|
| Rust (fmt / clippy / unit / integration) | success | 2m8s |
| Frontend (lint / typecheck / test / build) | success | 39s |
| Docker images and compose config | success | 2m10s |

即 v0.7.0 的远端门禁首次得到验证。docker job 是本机唯一无法复现的一个。

### 工具坑（写本文件时必读）

- 工具坑：apply_patch 在上一行以全角逗号结尾时会吃掉下一行的新增前缀
  - 现象：报 "invalid hunk at line N"，看起来像上下文没匹配上，实际是前缀被吃
  - 规避：写本文件时让每行都不以全角逗号 `，` 收尾，改用句号或分号
  - 同源坑：正文里出现连续两个 at 符号也会让解析器以为换了 hunk，报 End Patch 缺失

---

## ✅ v0.7.0 已发布（2026-10-02）

https://github.com/ZhongGheart/axum-api/releases/tag/v0.7.0

### 发布动作（全部已完成）

| 步骤 | 结果 |
|---|---|
| PR #6 squash 合并进 `master` | `66f26595` |
| tag `v0.7.0` 打在 merge commit 上 | `refs/tags/v0.7.0` → `66f26595` |
| Release 已发布 | 非 draft、非 prerelease |
| 删 `v0.7.0` 分支 | **未做**，待用户确认 |

tag 打在 merge commit 而非分支 tip，与 v0.5.0 / v0.6.0 一致。
`master` 原本停在 `d91dbbf0`（v0.6.0），现为 `66f26595`。

### 本轮又踩了一次 ref 歧义坑

`git push origin v0.7.0` 报 `refspec matches more than one`：
本地分支 `v0.7.0` 与新 tag `v0.7.0` 同名，git 拒绝猜测。
**必须写完整 refspec** 才推得上去：

```
git push origin refs/tags/v0.7.0:refs/tags/v0.7.0
```

这正是 v0.5.0 记过的坑（见 commit `cf8cbdde`）。当时只在文档里记了，
没形成操作纪律，这次又踩了一次。**结论：打与分支同名的 tag 时，
推送一律用完整 refspec，不用短名。**

### 合并前补做的一件事

合并前发现**本地门禁比 CI 窄**：CI 的 clippy / test 带
`--locked --all-targets --all-features`，而之前本地跑的不带
`--all-targets --all-features`（少了集成测试 target 与全 feature 组合）。
按 CI 原始命令重跑一遍后全绿，才推的 PR。

另外 `docker` job 本机无法复现（无 Docker daemon），只在 CI 上验证过；
它在 CI 上通过，所以分支的验证是完整的。

---

# v0.8.0 第 1 项 —— 菜单删除的授权下界（原计划前提被实测推翻）

## 计划前提被实测推翻

v0.7.0 留档的原计划是："`create_menu` 无授权下界，
持 `system:menu:create` 的角色可以声明一个后端认识的码再分发给别的角色"，
候选规则为"只允许声明当前没有任何菜单在用的码"。

**实测结论：那个洞其实早就被堵上了，堵的方式和预期不同；
而真正没被堵住的洞在 `delete_menu`。**

## 实测证据（三条探针，跑完已丢弃，不留在仓库）

### 探针 1：声明既有码会被数据库挡下

用 admin 建一个 `permission = system:user:delete` 的按钮 →
**500 Internal Server Error**，message 是"服务器内部错误"。

挡住它的是迁移 `007` 的部分唯一索引：

```
CREATE UNIQUE INDEX idx_menus_permission_unique
    ON menus (permission) WHERE permission IS NOT NULL AND permission <> '';
```

所以"声明一个既有码"在**数据库层**就被拒绝，不需要额外授权下界。
原计划设想的规则已经被索引免费提供了。

但代价是**入参错误被当成服务端故障**：该返回 400/409，实际返回 500。
这会污染错误监控，也让管理员看不懂发生了什么。

### 探针 2：create → delete → create 同一个码，三步全部 200

delete 释放了唯一索引占位，于是同一个码可以被重新声明。
**说明"唯一索引"这个下界可以被 delete 绕过**，它不是可靠的授权边界。

### 探针 3（决定性）：删除已授予角色的按钮会把码从那个角色身上剥掉

夹具：把一个临时码的按钮授予 carrier 角色；
另建 deleter 角色，**只持 `menu:list` + `menu:delete`**（不持该码，也无 `menu:grant`）。

```
carrier holds before = ["probe:revoke:02f64"]
deleter delete status = 200 OK
carrier holds after  = []
```

**持 `system:menu:delete` 的角色，对别的角色完成了一次跨角色撤权，
全程绕过 `system:menu:grant`。**

## 真正的问题：v0.7.0 修的是同一个洞的一半

| 入口 | v0.7.0 之后 | 后果 |
|---|---|---|
| `update_menu` 清空已授权按钮的码 | **要求持该码**（v0.7.0 新增守卫） | 安全 |
| `delete_menu` 删掉已授权按钮 | **无任何检查** | 同样的跨角色撤权 |

两条路径的**效果完全等价**（码从目标角色身上消失），
但只堵了前者。`delete` 是锤子更大的那把：连按钮行本身都没了。

自提权路径是闭合的：把码授予别的角色后，
`ensure_can_grant_roles` 要求调用者已覆盖目标角色的码，
所以把自己塞进那个角色会被拦下（已实测确认）。
因此这条是跨角色**撤权**，不是自提权——但撤权本身已经足够严重。

## 本项要做两件事

### 1. `delete_menu` 授权下界（真正的安全修复）

删除一个**携带权限码且已授予至少一个角色**的菜单时，要求持有该码。

- 复用 v0.7.0 已有的 `is_granted_to_any_role`，语义与 update 的守卫完全对齐
- 未授予任何角色的按钮照旧可删（与"清空无害"同理）
- 目录/页面菜单不携带码，不受影响（沿用 v0.5.0 既定判断）
- **副作用**：堵上探针 2 的 delete → recreate 绕行

### 2. 声明已被占用的码返回 409 而不是 500

把唯一索引冲突翻译成 `AppError::Conflict`。
这是既有契约的补齐，不是新规则。

## 起始 git 状态

- 分支 `v0.8.0`（从 master 切出）
- HEAD = `0715ece9 docs(handoff): 记录 v0.7.0 发布结果与 ref 歧义坑复发`
- 上一版：v0.7.0 已发布（tag `v0.7.0` → merge commit `66f26595`）
- `v0.7.0` 分支已删（远端与本地）

---

## v0.8.0 第 1 项完成记录（delete_menu 授权下界）

### 落地内容

- `MenuRepository::granted_codes_in_subtree(id)`：递归 CTE，取该菜单**整棵子树**里
 所有"已被授予至少一个角色"的权限码
- `MenuRepository::is_permission_taken(code)`：权限码占用预查
- `delete_menu` 装上授权下界，与 v0.7.0 的 update 守卫语义对齐
- `create_menu` 加占用预查；`MenuRepository::create` 把唯一索引冲突映射成 `Conflict`
- **计划外补的一项**：`menus_type_check`（`type` 非法值）原本也冒成 500。
 与权限码那条同源，现统一由 `map_write_violation` 翻译，覆盖 create/update 两条写路径。
 这是我在真实浏览器回归里撞出来的——脚本传了 `type: "page"` 拿到 500。
 不修的话等于把一个已知会误导管理员的 500 留在刚动过的函数里

### 一个设计要点：守卫不能把菜单锁死

`admin` 造出一个码后**并不持有**它，所以 admin 也不能直接删承载该码的菜单。
这不是缺陷，是规则的必然推论。出路是**先撤销授权、再删除**
（撤销需 `system:menu:grant`），菜单变成"没人依赖"后删除即放行。

既有三条 v0.7.0 用例的**清理步骤**正是在这里拿到 403——那是正确行为。
新增 `cleanup_temp_menu_dir` 走恢复路径清理，并把"没有锁死"变成了断言。

刻意**没有**给 admin 开后门：handoff 里既定的设计决定第 3 条
"admin 是数据上的超级用户，不是代码里的后门"仍然成立。

### 测试：集成测试 63 → 68（新增 5 条）

`deleting_a_granted_button_others_rely_on_requires_holding_it`、
`deleting_a_directory_with_a_granted_button_below_is_denied`、
`deleting_a_button_no_role_relies_on_is_allowed`、
`declaring_an_already_used_permission_code_is_a_conflict`、
`an_invalid_menu_type_is_a_bad_request`

### 缺陷注入验证（四条，含一条鉴别性结果）

| 注入 | 结果 |
|---|---|
| 移除 delete 守卫 | 两条删除用例同时变红 |
| 子树查询去掉递归 | 目录级联那条变红，直接删按钮那条**仍绿** |
| 移除占用预查 | 冲突用例**仍绿** |
| 预查与索引映射都移除 | 冲突用例变红 |
| 移除 `menus_type_check` 分支 | 类型用例变红 |

第二条证明递归查询真正承重，且两条用例覆盖**不同的**攻击路径
（删按钮 / 删父目录）。这是本轮最有价值的一次注入。

第三条暴露**诚实的覆盖缺口**：占用预查与唯一索引映射两层都报冲突，
测试只能区分"两层都没了"，即**预查本身没有被独立覆盖**。
预查负责的是可操作的消息文案，正确性由索引映射那层兜底。
记在这里以免后人误以为预查已被测试保护。

### 门禁结果（全绿）

- `cargo fmt --all --check` / `cargo clippy --locked --all-targets --all-features -D warnings`：clean
- 单元测试 55（未变，本项无纯逻辑单元可拆）
- 集成测试 63 → 68
- 前端代码未改动，但版本号有动，故仍跑了前端门禁：lint / typecheck / vitest 84 全绿

### 起始 git 状态

- 分支 `v0.8.0`（从 master 切出）
- HEAD = `0715ece9 docs(handoff): 记录 v0.7.0 发布结果与 ref 歧义坑复发`

---

## ✅ v0.8.0 已发布（2026-10-02）

https://github.com/ZhongGheart/axum-api/releases/tag/v0.8.0

### 发布动作（全部已完成）

| 步骤 | 结果 |
|---|---|
| PR #7 squash 合并进 `master` | `554859dc` |
| tag `v0.8.0` 打在 merge commit 上 | `refs/tags/v0.8.0` → `554859dc` |
| Release 已发布 | 非 draft、非 prerelease，且是 latest |
| 删 `v0.8.0` 分支 | 见下 |
| master CI | 三个 job 全绿 |

`master` 从 `0715ece9`（v0.7.0 发布记录）前进到 `554859dc`。

### ref 歧义坑这次没有复发

v0.7.0 发布时踩过一次：`git push origin v0.7.0` 报
`refspec matches more than one`，因为本地分支与新 tag 同名（记于 `cf8cbdde`）。
**这次打 tag 时直接用了完整 refspec，一次推成功**：

```
git push origin refs/tags/v0.8.0:refs/tags/v0.8.0
```

这说明上次把它写进文档是有用的——真正的修复是"形成操作纪律"，
而不是"再记一遍"。

### 本版的核心结论：计划阶段会记错，实测阶段才会发现

留档给下一项的 `create_menu` 授权下界（v0.7.0 写下、v0.8.0 计划照抄）
**是被实测推翻的**：那个洞早被迁移 `007` 的唯一索引堵住，
真正没堵的是 `delete_menu`。已把那段原文标注为"已被推翻，勿照此施工"，
而不是删掉——删掉的话，后人仍可能从别处的转述里捡起这个错误前提。

**教训**：授权类改动不要靠读代码推断"哪里没守"，先写探针把攻击链跑一遍。
本版的两个洞（delete 无守卫、delete 级联删子树）都是探针跑出来的，
不是看代码看出来的。



---

# v0.9.0 候选调查（本轮只调查，未动手）

## 调查方法：写探针，不读代码下结论

v0.8.0 的教训是"授权类改动不要靠读代码推断哪里没守"。本轮延续该纪律，
下面每条结论都来自对 8080 上运行实例的真实 HTTP 探测，不是静态阅读。

## 洞 A：`assign_user_role` 不检查**目标用户**（与 delete/update 不对称）

`POST /api/admin/users/{user_id}/roles` 只对**被授予的角色**跑
`ensure_can_grant_roles`，从不看**目标用户当前持有什么角色**。
`update_user` / `delete_user` / `batch_delete_users` 三处都查目标用户。

### 实测（弱角色只持 `system:user:update`）

```
弱角色 = probe_weak2（仅 system:user:update）
目标   = 一个只持 admin 角色的账号 admin2

弱角色给 admin2 追加 'user' 角色
  -> HTTP 200 角色分配成功
  -> admin2 的角色: ["admin"]  ==>  ["admin","user"]      <-- 写确实落库了
```

同一条弱角色、同一个目标，另外两个入口都拒绝：

```
弱角色整体替换 admin2 的角色为 [admin]
  -> HTTP 403 缺少权限：修改该用户需要「system:dict:create」
弱角色删除 admin2
  -> HTTP 403 缺少权限：system:user:delete
```

### 这条洞的实际影响有限，但不是零

- **不是自提权**：弱角色自己没被提权（实测其角色始终是 `[probe_weak2]`）。
  `ensure_can_grant_roles` 对被授予角色那道闸门是有效的，拦住了"授予高于自己的角色"
- **是跨账号的越权写**：一个只配了 `system:user:update` 的运维角色，
  能改一个权限远高于自己的账号的角色集合。这与 v0.5.0 PR-3
  在 delete/update 上确立的边界直接矛盾
- **净效果多半是"添弱角色"而非"剥离强角色"**（追加语义），
  故危害低于 v0.8.0 那个撤权洞，但它是同一类边界不一致

## 洞 B：追加角色后不撤销存量会话（新权限要重新登录才生效）

`update_user` 在角色集合变化时显式调用 `revoke_all_sessions`，
理由写在注释里："权限已变化：吊销存量会话，旧令牌不得继续携带旧角色"。
`assign_user_role` 改了同一份数据却没有这一步。

### 实测

```
目标 victim 持 [user]，令牌有效
  GET /api/admin/dict/types  -> 403
operator 追加 probe_grant（该角色含 system:dict:list）
  -> HTTP 200，victim 角色变成 ['user', 'probe_grant']
victim 用**原来那个令牌**再打
  -> 403        <-- 新权限没生效
victim 重新登录
  -> 200        <-- 重新登录后才生效
```

### 两次探针翻车记录（留着免得后人重犯）

1. 第一版探针把 `system:dict:create` 授出去，却去打 `GET /api/admin/dict/types`。
   该端点要的是 `system:dict:list`（`PermDictList`），所以 403 是**探针写错了**，
   不是产品的洞。差点误报
2. 第一次测洞 A 时目标 admin 账号**本来就已持有 `user` 角色**，
   而 `assign_role_to_user` 是 `ON CONFLICT DO NOTHING` ——
   于是 200 可能是**静默无操作**，证明不了写入发生。
   于是改用一个干净的 `admin2` 账号（只持 admin）重测，才看到
   `["admin"]` → `["admin","user"]` 的真实变化

结论：**判据必须落在数据变化上，不能落在状态码上。**

## 探针数据已清理

`victim_*` / `oper_*` / `pw2_*` / `admin2_*` 账号与 `probe_*` 角色全部删除，
库已回到 `users=[admin]`、`roles=[admin, user]`。

---

# 后续版本计划（按序，v0.8.0 已完成）

## v0.9.0（建议下一版）：`assign_user_role` 的两处不对称

把洞 A 与洞 B 放在同一版，因为它们是同一个端点的同两处缺失，
一次改完、一次发版。范围：

- 补目标用户检查（与 `update_user` / `delete_user` 对齐）
- 角色集合变化后吊销目标用户存量会话（与 `update_user` 对齐）
- 集成测试覆盖两条，且必须做缺陷注入验证
- 真实 Chrome 回归（这版改的是后端，前端不动，但接口行为变了）

## v0.10.0：浏览器回归脚本进仓库

`/tmp/axum-e2e/` 下 `regress.mjs`(14 项) + `delguard.mjs`(13 项) +
`cdp.mjs` + `monitor-page.mjs` 共约 22KB，**仓库内无 e2e 目录，会丢**。

价值依据：v0.7.0 CHANGELOG 自己写了"分页表格缺陷靠人工核对截图发现" ——
CI 的 3 个 job 全绿也发现不了这类 UI 回归。仓库内 `frontend/package.json`
无任何 e2e 依赖（无 playwright / cypress / puppeteer），
现状是 Node 22 内置 WebSocket 直连 CDP、零依赖，搬进仓库成本低。

## v0.11.0：授权探针工具化

本轮与 v0.8.0 的洞**都是探针跑出来的**，不是读代码看出来的。
47 个 handler 逐个手写探针不现实，应做一个可复用的
"写路径守卫覆盖"探针脚本，把偶然变常规。

## 待产品决策，不要塞进安全版本

- **数据范围 / 行级过滤缺失**（`list_all` 返回全部用户）：这是**功能缺口不是 bug**，
  属产品决策（要不要按部门/创建人过滤），不该混进安全加固版本
- **Prettier 未进 CI**：50 个前端文件风格不统一，一次性重排会淹没真实改动；
  老问题，价值低于上面几项

## 次要观察（未确认是否值得修）

`assign_role_to_user` 用 `ON CONFLICT DO NOTHING`，重复授权返回 200
"角色分配成功"但实际什么都没写。幂等语义本身没错，
只是返回值在重复调用时略有误导。低危。

---

# v0.9.0 — 授权边界补齐 + 回归资产入仓 + 探针工具化（三项合并）

## 当前目标

把原先排成 v0.9.0 / v0.10.0 / v0.11.0 的三项工作**合并为同一个 v0.9.0** 一版发：

1. `assign_user_role` 的两处不对称（目标用户检查 + 存量会话吊销）
2. 浏览器回归脚本从 `/tmp/axum-e2e/` 搬进仓库
3. 授权探针工具化（可复用的"写路径守卫覆盖"探针）

三项工作性质不同（安全修复 / 测试资产 / 工具），但都在同一条
"把偶然变常规"的主线上，且 2、3 恰好是验证 1 的基础设施：
探针脚本会长期留在仓库里，回归脚本也会成为后续每版的验收手段。

## 当前计划

| 步骤 | 内容 | 状态 |
|---|---|---|
| 0 | 记录目标与起始 git 状态 | ✅ |
| 1 | 洞 A：`assign_user_role` 补目标用户检查 | ✅ 里程碑 1 |
| 2 | 洞 B：角色变化后吊销目标用户存量会话 | ✅ 里程碑 1 |
| 3 | 洞 A/B 的集成测试 + 缺陷注入验证 | ✅ 里程碑 1 |
| 4 | 回归脚本搬进仓库 `e2e/`（去绝对路径依赖） | ✅ 里程碑 2 |
| 5 | 授权探针工具化 | ✅ 里程碑 3（并抓到第四个洞：`delete_role` 天花板） |
| 6 | 真实 Chrome 全量回归 | ✅ 里程碑 2 / 4 |
| 7 | 质量门禁（fmt/clippy/单元/集成/前端/e2e/探针） | ✅ 里程碑 4 |
| 8 | 文档：CHANGELOG / README / 版本号 | ✅ 里程碑 4 |
| 9 | 合并 / tag / Release | 待做（**等用户指令才推送**） |

## 起始 git 状态

- 分支：`master`（与 `origin/master` 同步）
- HEAD = `fbf301ed docs(handoff): 记录 v0.8.0 发布结果与 ref 歧义坑未复发`
- 工作区：仅 `docs/AI_HANDOFF.md` 有未提交改动（上一轮调查结论）
- 版本号：Rust / frontend 均 `0.8.0`

## 关键决策

1. **三项合并发版，不拆**。用户明确要求。理由站得住：
   探针工具与回归脚本是验证洞 A/B 的长期资产，拆开发它们等于
   连续三版都在改测试基建却没拿到安全修复
2. **洞 A 的修法与 `update_user` 对齐**：加目标用户检查，
   但**不阻断"给自己追加弱角色"**——那是合法的自我降级路径，
   拦它反而会让运维无法给自己减权
3. **洞 B 与 `update_user` 对齐**：角色集合真变了才吊销会话。
   重复追加同一角色是幂等的（`ON CONFLICT DO NOTHING`），
   那种情况不该踢掉目标用户


---

## 里程碑 1：洞 A / B + 第三个洞已修复并验证（步骤 1-3 完成）

### 改了什么

| 文件 | 改动 |
|---|---|
| `src/controller/role.rs` | `assign_user_role` 加三道守卫：目标用户存在性、目标用户权限下界、角色真变了才吊销会话 |
| `src/utils/jwt.rs` | `Claims` 新增 `iat_ms`（毫秒签发时间，`serde(default)` 兼容旧令牌） |
| `src/utils/redis.rs` | `revoke_user_sessions` 的水位由**秒**改**毫秒** |
| `src/middleware/auth.rs` | 吊销比对改用 `iat_ms` |

### 挖坑记录：修洞 B 时掉进了一个更深的坑

第一版修法是「存 `now + 1` + 严格小于」，以为这样既能堵住同秒窗口、
又不误伤新登录。**实测打脸**：测试在 fail 与 pass 之间跳。
根因不是我的守卫写错，而是 **JWT 标准的 `iat` 只有秒级精度**（RFC 7519）——
令牌在 `10:00:00.100` 签发、吊销发生在 `10:00:00.900`，两者 `iat` 都是
`10:00:00`，**信息已经丢了**，秒级方案只能二选一，两个都是错的：
要么放过旧令牌（漏吊销），要么误伤同一秒内新登录的令牌。

只有把精度提到毫秒才有两个选项之外的办法，故新增 `iat_ms`。
旧令牌（升级前签发）没有该字段，`serde(default)` 给 0，
0 一定小于任何吊销水位 —— 方向是**失效**而非放行，升级后首次吊销
会把存量令牌一并作废，这是安全的一侧。

### 测试：集成测试 65 → 70（新增 5 条）

- `appending_a_role_to_a_stronger_account_is_denied`（洞 A）
- `appending_a_weaker_role_to_yourself_is_still_allowed`（洞 A 的镜像：合法路径不许被误伤）
- `appending_a_role_takes_effect_without_waiting_for_a_relogin`（洞 B）
- `re_applying_the_same_role_does_not_kill_the_target_session`（洞 B 的配套：幂等不踢人）
- `appending_a_role_to_an_unknown_user_is_not_found`（第三个洞）

单元测试 55 未变（本次无纯逻辑可拆）。

### 缺陷注入验证（5 条，全部真实变红后还原）

| 注入 | 结果 |
|---|---|
| 移除目标用户下界 | 洞 A 用例变红 |
| 移除会话吊销 | 洞 B 用例变红 |
| 幂等判断改成"总是吊销" | 幂等用例变红 |
| 移除目标用户存在性检查 | 404 用例变红（回到 500） |
| **吊销水位退回秒级** | **洞 B 用例变红，且失败形态是 `401` vs `200`** |

最后一条最有价值：它证明毫秒化是**承重**的，且暴露的正是
"误伤同一秒内新登录的令牌"这个失败模式——不是"漏吊销"，
说明秒级方案的两个错误方向都被这条用例覆盖到了。

### ⚠️ 自己踩的坑：一条测试污染共享库，连带 19 条误报

第一次跑全量时 20 条失败，看起来像改动炸了一片。
先按 handoff 纪律**整库重建**——没用；再 `git stash` 回到干净 master 跑——
**65 条全绿**，确认是我的改动。

真因在测试夹具，不在产品代码：我那条「给更强账号追加角色」的用例
给目标账号塞了**字面意义上的 `admin` 角色**，在共享库里留下第二个管理员。
`ensure_not_last_admin` 只拒绝"降级最后一个管理员"，既然还剩一个，
降级 `admin` 就是合法的 → `last_admin_cannot_be_demoted_or_deleted` 失败
→ admin 权限被摘 → 后面 19 条用 admin 令牌的用例全部 403。

改法：目标改用**自定义强角色**（持 `system:role:delete`），
既能验证同一条目标下界，又不留脏数据；并在用例内显式清理。
教训：**共享测试库里"多留一个管理员"会污染看似无关的一大批用例**，
排查时要先怀疑夹具，再怀疑产品代码。

**门禁**：单元 55 ✅ / 集成 70 ✅（clean master 为 65，+5 为本版新增）


---

# 功能缺口调查（2026-10-02，v0.9.0 途中）

## 调查方法：探针 + 前后端对账，不靠印象

沿用 v0.8.0 的纪律。下面每条都有实测或源码对账支撑，
并且**区分"UI 承诺了但后端没实现"和"压根没做"**——前者更糟，
因为界面在骗人。

## 缺口一：用户列表的搜索框是假的（UI 承诺，后端丢弃）

- 前端 `frontend/src/views/system/user/index.vue:247` 渲染了 `<SearchForm>`，
  但 `onSearch(_keyword: string)` **把关键字丢掉了**，只重置页码后重新拉全量
- 后端 `UserListParams` 只有 `page` / `page_size` 两个字段，没有关键字
- 实测四种拼写全部无效，`total` 恒为 52：
  `?keyword=admin` / `?username=admin` / `?search=admin` / `?role=admin`

**危害不在功能缺失，在于界面骗人**：搜索框能点、能输、回车没反应。
用户会以为系统坏了或自己操作错了，而不是"这功能没做"。

## 缺口二：审计日志的筛选栏也是假的（同一个毛病的另一处）

- 前端 `views/system/log/index.vue` 有 `filters.action` / `filters.username`
  两个输入框，并且**真的把它们发给后端**（`auditApi.list({...filters})`）
- 后端 `demo::list_audit_logs` 的入参是 `PaginationParams`，
  只有分页与排序字段，`action` / `username` 被 serde 静默丢弃
- 实测 `?user_id=x&action=login&method=POST&from=2020-01-01` 全部无效

比缺口一更糟：查审计日志恰恰是"出事了要查"的时候，
筛选栏在那儿但不起作用，会把人引向错误的结论。

**两条其实是同一类缺陷**：前端承诺筛选、后端没有。
修法也同形（后端加过滤 + 前端把关键字传下去），故合并成一版做。

## 缺口三：没有自助改密

实测 `POST /api/auth/change-password`、`PUT /api/auth/password` 均 404。
唯一的改密入口是 `PUT /api/admin/users/{id}/reset-password`，
需要 `system:user:update`。

即：**非管理员永远无法改自己的口令**，只能找管理员重置。
前端也没有个人中心——顶栏下拉里只有「退出登录」。

## 缺口四：公开注册无审批、无邮箱验证

实测未认证连续注册 12 次全部 200。注册出的账号：

- 角色固定为内置 `user`（`service/auth.rs:79` 硬编码）
- 实测持有权限码 `[]`，打 `/api/admin/users`、`/api/admin/menus` 均 403

**必须诚实说明：这不是提权漏洞。** 它是资源占用 / 垃圾账号问题，
不是越权。而且全局限流仍在（`RATE_LIMIT_IP_MAX=100` 次/分/IP），
所以并非真正"无限"，只是阈值偏松。

真正的问题是**产品决策**：一个内部管理后台是否需要公开注册入口？
关掉它比加固它更合理。这是决策题，不是工程题。

## 缺口五：角色列表无分页

实测 `GET /api/admin/roles` 一次性返回全部（当时 36 个），
`?page=1&page_size=1` 不生效。当前规模无痛，角色多了就是全表传输。

## 已知但仍不做的（此前已记录，结论不变）

- 数据范围 / 行级过滤：功能缺口，属产品决策
- Prettier 未进 CI：老问题，价值低于上面几条

---

# 后续版本计划

## 首要：先收尾 v0.9.0（当前半成品，不可跳过）

v0.9.0 = 三项合并（授权边界补齐 + 回归资产入仓 + 探针工具化）。
现状：

| 步骤 | 状态 |
|---|---|
| 1-3 洞 A/B/第三个洞 + 测试 + 注入验证 | ✅ 完成（集成 65 → 70） |
| 4 回归脚本入仓 | 🟡 **一半**：`lib/cdp.mjs`、`lib/harness.mjs`、`suites/permission-and-monitor.mjs`、`suites/menu-delete-guard.mjs` 已写；**缺第 3 个套件**（角色追加守卫）、`run.mjs`、`README.md`、`.gitignore` |
| 5 授权探针工具化 | ❌ 未开始 |
| 6 真实 Chrome 回归 | ❌ 未跑（套件还没写完） |
| 7 质量门禁 | ❌ 未跑 |
| 8 文档 / 版本号 | ❌ 未做 |
| 9 合并 / tag / Release | ❌ 未做 |

**不要在 v0.9.0 半成品状态下去开新版本**——上一轮就因为测试夹具污染
共享库导致 20 条误报，教训是先把一版关干净再开下一版。

## v0.10.0：把"假筛选"变成真筛选（缺口一 + 二）

两处同形缺陷合并一版。后端加过滤 + 前端把关键字传下去，
并补集成测试证明过滤真的生效（判据落在返回条数上）。

顺带把角色列表分页做掉（缺口五），它很小，一起发。

## v0.11.0：自助改密 + 个人中心（缺口三）

新端点 + 新页面。注意两个既有约定：
- 改密后必须吊销存量会话（与 v0.9.0 的 `iat_ms` 精度对齐）
- 不能让用户改自己的 `is_active` 或角色

## 待产品决策，先不动

- **缺口四（公开注册）**：建议直接关闭公开注册，改为管理员建号。
  这是配置/产品决策，不该由我默认决定
- 数据范围 / 行级过滤
- Prettier 进 CI


---

# 里程碑 2：浏览器回归脚本进仓 + 真实 Chrome 跑通（步骤 4、6 完成）

## 落地的东西

```
e2e/lib/cdp.mjs                        最小 CDP 驱动（零依赖，Node 22 内置 WebSocket）
e2e/lib/harness.mjs                    公共夹具（起浏览器/登录/多身份 api/截图/汇总）
e2e/suites/permission-and-monitor.mjs  v0.7.0 权限码+监控导出
e2e/suites/menu-delete-guard.mjs       v0.8.0 菜单删除守卫
e2e/suites/role-assignment-guard.mjs   v0.9.0 角色追加守卫（本次新增，24 条断言）
e2e/run.mjs                            套件 runner（独立进程，逐个汇总）
e2e/README.md                          怎么起服务、怎么跑、怎么加套件
```

`.gitignore` 加了 `e2e/.artifacts/`（截图、下载、Chrome 临时 profile，每次重生成）。

原脚本在 `/tmp/axum-e2e/`，配置全是硬编码路径；进仓时全部改成环境变量
（`E2E_APP` / `E2E_CDP_PORT` / `E2E_ARTIFACTS` / `CHROME_PATH` / `E2E_USER` / `E2E_PASS`），
并让 harness 自己拉起 headless Chrome：若 9222 上已有浏览器就复用，
且**只关自己拉起来的那个**，不误杀开发者正在用的窗口。

**零依赖**：没有 `package.json`，不需要 `npm install`。整个 e2e 只需要
导航/求值/截图/监听网络四件事，为它拉进 ~300MB 的 Playwright 不划算。

## harness 的两处补充

### 1. `apiAs` / `tokenFor`：跨身份

套件 3 的核心是跨身份（管理员建号 → 弱操作员尝试越权 → 目标用户验会话），
而原 `api()` 只认浏览器里那一份 admin 令牌。新增：

- `tokenFor(user, pass)`：换个身份拿令牌，**不写 localStorage**
- `apiAs(token, method, path, body)`：用指定令牌发请求
- `currentToken()`：读浏览器当前令牌

不去改写 localStorage，是为了不搅乱 admin 的浏览器会话——
界面登录验的是"人能点"，脚本换身份验的是"接口认谁"，两件不同的事。

### 2. `checkNoConsoleErrors(expected)`：故意造出来的失败不该算缺陷

原版 `checkNoConsoleErrors()` 不带参数，把套件**故意**触发的 403 也算成错误，
于是每个套件都必须在"自己的核心断言"和"这条检查"之间二选一。
改为与 `checkNoUnexpectedHttp` 共用同一份预期清单，
只放行清单里那几个状态码的 `Failed to load resource` 网络日志，其他一律算错误。

配套加了 `forgetDeliberateFailures()`（网络 + 控制台一起清）。
**为什么不直接把 401 加进白名单**：加进白名单后，本套件里**任何** 401
都会被当成"预期"，而"令牌意外过期""会话被误吊销"恰好都表现为 401，
正是它要抓的那类回归。就地划掉只豁免那一次，余下阶段 401 仍是硬信号。

## ⚠️ e2e 首次跑就抓到一个假绿：后端二进制是旧的

套件 3 第一次跑，`[4] 重新登录后新权限可用` 报 **401**（期望 200）——
新令牌也被判失效，正是"误伤新登录"方向。

第一反应是"毫秒化没生效，产品有 bug"。但源文件里 `claims.iat_ms < ts` 明明白白。
解码真实令牌确认 `iat_ms = 1790938265994`（毫秒，量级正确），
再查 Redis 里该用户的水位 = `1790938241076`（也是毫秒）。数值都在同一量级。

手工 curl 复现，比对三个数：

```
watermark  = 1790938337433   (吊销时刻)
T2.iat_ms  = 1790938337922   (吊销**之后**签发，晚了 489ms)
T2 -> 401                   但 1790938337922 < 1790938337433 为假，不该被拒
```

结论：运行中的中间件**仍在用旧的 `claims.iat < ts`**（秒 vs 毫秒）。
秒级 `iat`（~1.8e9）永远小于毫秒水位（~1.8e12），
于是**所有**令牌一律被判吊销——包括刚签发的。

真因：`target/debug/axum-api` 与 `src/middleware/auth.rs` 的 mtime 撞在同一分钟
（18:32），二进制是上一次构建的产物，只编进了同分钟里更早改的 `role.rs`
（所以之前那条 404 探测是过的，给了我"二进制没问题"的错觉）。
`cargo build` 重启后复现即消失。

**教训**：
1. mtime 撞在同一分钟时不可信，别用它判断"二进制新不新"——要么重建，要么探针验行为。
2. 一次通过的探针只能证明**它验的那一处**。404 通过并不能推出 `auth.rs` 也编进去了。
   这也是为什么后来改用"逐个行为探针"而不是"看时间戳"。
3. e2e 这层抓到的第一个问题就是**环境问题**而非产品缺陷。真实浏览器回归的价值
   在于它跑的是**部署物**（编译产物 + dev server + 数据库），比单元测试更贴"上线那一刻"。

## e2e 的承重验证（缺陷注入）

把水位和比对**一起退回秒级**并重新构建，套件 3 精准变红：

```
FAIL  追加后旧令牌立刻失效（会话已吊销）  ::  status=403 缺少权限：system:dict:list
FAIL  失效形态是 401 而不是 403（否则等于没吊销）  ::  status=403
```

失败形态是 **403 而非 401** = 旧令牌被**放过**（漏吊销方向）。
其余 22 条仍绿——说明注入只打掉了洞 B，没有连带污染别的断言。
已还原，`grep INJECT` 无残留。

注意这次失败形态与集成测试里那条注入**相反**（集成侧是 `401` vs `200`，
暴露"误伤新登录"；e2e 侧是 `403`，暴露"漏吊销"）。
两层看到的不是同一个 bug 侧面，都得留着。

## 自己在这层踩的三个坑

1. **`api()` 忘了 `await`**：改成 `return this.apiAs(this.currentToken(), ...)` 后
   令牌是 Promise，`JSON.stringify` 序列化成 `{}`，header 成了 `Bearer {}` → 全线 401。
   14 条断言一起变红，看着像"改动炸了一片"。
2. **中文提示词被拼成裸代码**：`throw new Error(` + JSON.stringify(username) + ` 登录失败: ` + ...)
   生成的是 `new Error("user" 登录失败: + res.status)`，页面直接报语法错。
   提示词要整段带引号。教训：拼 JS 字符串时，先 `new Function` 解析一遍再送进浏览器。
3. **Vite 只绑 IPv6 `::1`**，而 harness 默认写死 `127.0.0.1`，连不上且报错像"服务没起"。
   默认值改成 `localhost`（两种栈都解析），README 同步注明。

## 门禁（e2e 层）

```
permission-and-monitor.mjs   13/13 ✅
menu-delete-guard.mjs        14/14 ✅
role-assignment-guard.mjs    24/24 ✅
runner                       3/3 套件通过 ✅
```

环境：后端连 `axum_api_test`（55432/56379）跑在 8080，Vite 在 3000，
Chrome 154headless，探针确认运行实例含 v0.9.0 全部三处修复。

---

# 里程碑 3：授权探针工具化 + 抓到第四个洞（步骤 5、7 完成）

## 探针工具化：`e2e/probe-write-guards.mjs`

把"每个写入口都查了授权下界"从**纪律**变成**工具**。

此前每一版的洞都是探针跑出来的，但探针一直是一次性手写的。47 个 handler
逐个手写不现实，于是每版都靠"记得检查"——而"记得"正是会漏的那一环。

### 核心设计：清单不靠人列，从 OpenAPI 自动发现

这是本工具与一次性脚本的根本差别。新加一个 `POST /api/admin/xxx`，
它自动进入探针视野；**没登记探测方式就直接报"未覆盖"**，
逼着人当场决定，而不是留到下一版才发现漏了。

当前发现 24 个 `/api/admin` 写入口，全部有登记或明确豁免。

### 判定标准是一个可证伪的命题

> 只持入口所需**最小权限码**的操作员，对"权限高于自己"的目标发起写操作
> → 必须被拒（403），且数据不得有任何变化

**为什么必须用"最小权限码"**：若给操作员发 admin，被拒只可能来自目标下界检查，
测不出"入口权限码本身够不够"的真实边界，且权限码被调松时探针会误报通过。

### 每个条目三条断言，缺一不可

1. **弱操作员被拒**（403）
2. **数据侧未变**（403 也可能只是响应被拒、库已写脏）
3. **admin 做同一件事仍然成功**

第 3 条对破坏性入口尤其重要：**只测拒绝侧的话，"把入口整个禁掉"也能全绿**。
为此每次断言都用**专用靶子**，避免 admin 那次镜像真的把共享夹具删掉、
连带后面所有条目失真。

### `callApi` 统一处理 429

IP 限流 100/分，探针一轮必然超。处理放在 `callApi` 里而不是调用点，
否则每条目都要重复一遍、且漏一处就是一条假红。

## 探针第一次运行就抓到 v0.9.0 的第四个洞

只持 `system:role:delete` 的操作员，能删掉一个承载 `system:log:list` 的角色
（实测 **200，角色真被删**）。那个码他自己并不持有。

于是"只能授予/撤销自己已持有的权限码"这条不变量，在删除这条路上是失效的。
——与 v0.8.0 的 `delete_menu` 同一形状：**授权的两面只装了一面**。

### 修法与顺序

按 `delete_menu` 的同一模式补上天花板（`codes_of_roles` + `ensure_covers`），
但守卫顺序是**内置角色检查之后**，这是有意的：

内置角色永不可删是与权限无关的**固有事实**。若先报"缺少权限：X"，
操作员会误以为拿到 X 就能删内置角色——那是在把人往错误的方向引。
鉴权该早于的是"这个角色有几个人在用"这类**随调用者而变**的信息，
它仍在下面那句校验之前。

不像 `delete_menu` 那样要判断"是否有人依赖"：角色只要存在，
它携带的码就都在生效，删掉必然改变每个人的权限。

## ⚠️ 操作行为变化（必须向用户明示）

现在删除一个承载"你未持有的码"的角色会被拒。**删带自定义码的角色必须先撤授权。**

副作用：admin 自己造的码分发给别的角色后，admin 反而删不掉那个角色了。
这与 v0.8.0 的菜单删除同一性质——v0.8.0 的 e2e 里 admin 同样被拒过。
是既有设计的延续，不是新引入的意外。

## 测试：集成测试 65 → 73（本轮新增 3 条）

- `deleting_a_role_that_carries_permissions_you_lack_is_denied`
- `deleting_a_role_whose_permissions_you_cover_is_allowed`
- `deleting_a_role_that_carries_no_permission_is_allowed`

均通过缺陷注入验证：移除守卫 → 探针 39/41，两条精准变红。

### 对既有测试的影响（改的是清理，不是断言）

`role_menu_query_keeps_grants_whose_ancestors_are_not_granted` 的清理必须改为
**先撤授权再删角色**，因为该测试角色带自造的 `tmp:test:*` 码，admin 不持有它。
已在该处加断言把这个约束钉住——**否则将来有人会把清理改回去**，
且改回去时测试仍然通过，只是数据越留越多。

## 门禁（步骤 7）

| 项 | 结果 |
| --- | --- |
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | ✅ |
| 单元测试 | ✅ 55 |
| 集成测试 | ✅ 73（clean master 65，+8） |
| e2e runner | ✅ 3/3 套件 |
| 授权探针 | ✅ 41/41 |
| 前端 lint/typecheck/test/build | 本轮前端零改动，不受影响 |

## 剩余工作（本会话接手时）

1. 里程碑 4：文档与版本号（CHANGELOG / README / `0.8.0` → `0.9.0`）
2. 全量门禁复跑一次（改版本号后）
3. 提交，**不推送**（用户指令：全部工作完成或收到指令才统一推送）
4. 合并 / tag / Release

---

# 里程碑 4：版本号 / 文档 / 全量门禁（步骤 8 完成）

## 改动

| 文件 | 改动 |
|---|---|
| `Cargo.toml` / `Cargo.lock` | `0.8.0` → `0.9.0` |
| `frontend/package.json` | `0.8.0` → `0.9.0`（`pnpm-lock.yaml` 不记录根包版本，无需改） |
| `CHANGELOG.md` | 新增 `## [0.9.0]` 段 |
| `README.md` | 顶部导语补 v0.9.0、项目结构补 `e2e/`、门禁段补 e2e 与探针、新增「从 v0.8 升级到 v0.9」 |
| `e2e/README.md` | 补限流放宽说明 + 探针用法（见下） |
| `src/docs/mod.rs` | OpenAPI 版本号改为从 Cargo 派生 |

## 顺手抓到并修掉的：OpenAPI 版本号已漂移四版

对账时发现 `/api/openapi.json` 仍报 `0.4.0`——`src/docs/mod.rs` 里
手写的 `version = "0.4.0"`，**从 v0.5.0 到 v0.8.0 连续四版都没跟着
`Cargo.toml` 走**，没有任何测试或门禁发现。

改为 `version = env!("CARGO_PKG_VERSION")`。凡是"该跟着别处走"的值，
就不该有两份；这也是 v0.9.0 探针那条"清单不靠人列"思路的同一个道理。

**为什么之前没被抓到**：CHANGELOG 的双向覆盖测试只校验路由与文档同步，
不校验版本号。版本号这种"没人测、但会悄悄过期"的值，靠人记是记不住的。

## e2e 首跑假红：限流把套件打挂了（不是产品缺陷）

第一次跑 `node e2e/run.mjs`，套件 3 有两条 FAIL，全是 **429**：

```
FAIL  无控制台错误  ::  Failed to load resource: 429 (Too Many Requests) ×5
FAIL  除预期的 403/404 外无 4xx/5xx  ::  429 /api/admin/users ...
```

原因是**我自己的执行顺序**：探针刚跑完（它自己会等限流窗口滑过），
紧接着就跑了套件，IP 桶（100 次/分）在同一个窗口内被打满。

这不是产品缺陷——限流在正常工作。但它暴露了一个真实问题：
**套件的结论依赖"跑之前跑过什么"**，这让 e2e 不可信。
所以修在文档与启动配置上，而不是把 429 加进白名单：

- `e2e/README.md` 的启动步骤要求放宽 `RATE_LIMIT_IP_MAX` / `RATE_LIMIT_USER_MAX`
- 加了显式警告：否则会以 429 的形式假红

放宽后重跑：**探针 41/41 + 套件 3/3（13 + 14 + 24）全绿**，
且探针耗时从 ~70s 降到 18s（不再需要等窗口滑过）。

**为什么不加白名单**：429 加进预期清单后，这套 e2e 就再也不能发现
"限流阈值配错了"这类问题。加白名单是让测试说谎，环境要对齐。

## 行为探针（不信 mtime）

按上轮教训，重建二进制后不靠 `mtime` 判断"跑的是不是新代码"，
而是逐条打行为探针：

| 探针 | 结果 |
|---|---|
| OpenAPI 版本 = 0.9.0 | ✅（顺带证明 `docs/mod.rs` 编进去了） |
| 令牌含 `iat_ms` 且 `iat_ms // 1000 == iat` | ✅ 证明 `jwt.rs` 编进去了 |
| 给不存在的用户追加角色 → 404 而非 500 | ✅ 证明 `role.rs` 编进去了 |
| 探针实测删除承载自定义码的角色 → 403 | ✅ 证明 `delete_role` 天花板在运行实例里 |

## 全量门禁（最终）

| 项 | 结果 |
|---| --- |
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | ✅ |
| 单元测试 | ✅ 55 |
| 集成测试 | ✅ 73 |
| e2e 真实 Chrome 回归 | ✅ 3/3 套件（13 + 14 + 24） |
| 授权探针 | ✅ 41/41（24 个写入口全部有登记或豁免） |
| 前端 lint / typecheck / test / build | ✅（lint 1 条既有 warning：0 error） |

跑集成测试前**必须整库重建**并停后端，否则报"being accessed by other users"。

## 当前 git 状态

- 分支 `master`，HEAD 仍 `fbf301ed`（v0.8.0），**未推送**
- 版本号已 `0.9.0`
- 按用户指令：**本地跑完门禁即提交，全部工作完成或收到指令才统一推送**
- 下一步：提交（本里程碑），然后合并 / tag / Release（步骤 9）

---

# ✅ v0.9.0 已提交（2026-10-02，**未推送**）

提交：`440b27cb` —— v0.9.0：授权边界补齐（目标下界 / 毫秒吊销 / 删除角色天花板）
+ 回归与探针入仓。基线 `fbf301ed`（v0.8.0）。

改动 21 个文件（+2960 / -12）：4 处安全修复、e2e/ 8 个新文件（1403 行）、
文档与版本号、集成测试 +8 条。

## 门禁（提交前全绿）

| 项 | 结果 |
|---|---|
| fmt / clippy(-D warnings) | ✅ |
| 单元测试 | ✅ 55 |
| 集成测试 | ✅ 73 |
| e2e 真实 Chrome | ✅ 3/3 套件（13 + 14 + 24） |
| 授权探针 | ✅ 41/41（24 个写入口） |
| 前端 lint / typecheck / test / build | ✅ |

## 剩余步骤（等用户指令才推送）

1. `git push origin master`
2. `git tag v0.9.0 && git push origin v0.9.0`
3. GitHub Release（沿用前几版格式）

## 下一版：v0.10.0（两处"假筛选"变真筛选）

按用户要求，v0.10.0 / v0.11.0 的内容已合并进 v0.9.0；后续版本重新排期为：

### v0.10.0 — 把"假筛选"变成真筛选（缺口一 + 二 + 五）

| 缺口 | 性质 | 判据 |
|---|---|---|
| 用户列表搜索框丢弃 keyword | UI 骗人 | 传关键字后返回条数变少 |
| 审计日志筛选栏被 serde 静默丢弃 | UI 骗人 | 同上，且**不能静默** |
| 角色列表无分页 | 小，随规模恶化 | — |

两处假筛选**同源**：前端提交了参数，后端 DTO 没接（`serde` 默认忽略未知字段，
不报错也不生效）。修的时候要一并处理"静默丢弃"这个更隐蔽的毛病——
否则将来新增筛选条件仍会悄无声息地失效。

### v0.11.0 — 自助改密 + 个人中心（缺口三）

两个既有约定必须遵守：
- 改密后**吊销存量会话**（与 v0.9.0 的 `iat_ms` 精度对齐）
- **不能**让用户改自己的 `is_active` 或角色

### 待产品决策，不要塞进安全版本

- **缺口四：公开注册无审批、无邮箱验证**——建议关闭公开注册改为管理员建号，
  但这是产品/配置决策，不该由我默认决定
- 数据范围 / 行级过滤（`list_all` 返回全部用户）
- Prettier 进 CI（老问题，价值低于上面几项）

## 跨版本沉淀下来的三条纪律

1. **能自动发现的清单不要人列**。写入口清单、权限码清单都改成从 OpenAPI /
   `src/model/permission.rs` 派生，"记得检查"正是会漏的那一环
2. **同一件事的每个入口都要查**。v0.7.0 → v0.8.0 → v0.9.0 连续三个洞全是
   "授权的两面只装了一面"。新增写入口时问一句：**它等价于什么已有操作的镜像？**
3. **部署物的新旧要用行为探针验，不要看 mtime**。撞同一分钟时不可信，
   一次通过的探针只能证明它验的那一处

---

# ✅ v0.9.0 已发布（2026-10-02）

- Release: https://github.com/ZhongGheart/axum-api/releases/tag/v0.9.0
- tag `v0.9.0`（**附注标签**，与 v0.2.0 起各版一致）打在 `14d6ca25`
- `master` = `14d6ca25`，已推送，本地与 `origin/master` 同步（0 提交待推）
- 两个提交：`440b27cb`（v0.9.0 本体）+ `14d6ca25`（handoff 记录）

## ref 歧义坑第三次：这次没有复发

`git push origin v0.9.0` 会踩本地分支与 tag 同名的坑（v0.5.0 记过、
v0.7.0 又踩了一次）。本轮**事先查了** `git branch --list '*v0.9.0*'`——
本地并没有 `v0.9.0` 分支，即便如此仍按记下的纪律用完整 refspec：

```
git push origin refs/tags/v0.9.0:refs/tags/v0.9.0
```

**纪律**：打与分支同名的 tag 时，推送一律用完整 refspec，不用短名。
且动手前先确认是否真有同名分支，别凭记忆判断。

## tag 打在 HEAD 而非分支 tip

与 v0.5.0 / v0.6.0 / v0.8.0 的做法一致。本轮没有走 PR/合并（用户直接指示
推送 + 打 tag），tag 就落在包含 handoff 记录的最后一个提交上，核验过
`git rev-list -n1 v0.9.0 == git rev-parse HEAD`。

## Release 正文取自 CHANGELOG 对应段落

与 v0.8.0 同一做法：取 `## [0.9.0]` 段落去掉标题行，前面加一段摘要与
**升级前必读的「⚠️ 行为变化」提示**——因为本版有三处会改变现有运维动作的行为，
不提前说清楚会踩坑。

## 发布前本地门禁已按 CI 原始 flag 复核

v0.7.0 记过"本地门禁比 CI 窄"的坑。本轮本地跑的即 CI 原始命令：
`cargo fmt --all --check`、`cargo clippy --locked --all-targets --all-features
-D warnings`、`cargo test --locked --all-targets --all-features`、
`cargo test --test api_integration -- --ignored --test-threads=1`，
外加 CI 不跑的两层（e2e 真实 Chrome、授权探针）与前端四项。

## 一条自我纠正的记录

我原本在上一节写了"遗留：远端仍有 `v0.7.0` 分支，建议清掉"，
理由是"历史发布分支与同名 tag 并存正是 ref 歧义坑的成因"。**动手前实测，
发现这条是错的**：

```
$ git ls-remote --heads origin
14d6ca25...	refs/heads/master
```

远端**只有 `master` 一个分支**，`v0.7.0` 分支早已不存在。
v0.7.0 那轮记的"删分支：未做，待用户确认"后来实际已处理，
我在读旧记录时把"当时未做"当成了"至今未做"。

**教训**：交接日志记的是**当时的快照**，不是现状。
照着旧记录下结论前必须实测一次——这次若不查，就凭空给用户派了个不存在的活。

---

# 功能缺口复查（2026-10-02，v0.9.0 发布后）

不是重读上一轮那份清单，而是**重新对着代码查了一遍**，找**证据**。
本节取代上文「功能缺口调查」的排期结论，但缺口一/二/三/五的编号沿用，便于对照。

## 结论先行：还有一类缺口，前几轮没归过类——**界面在说谎**

前三版的洞是"授权的两面只装了一面"。这次查出一模一样形状的**另一族**问题：
**前端提供了控件，后端没有对应能力，且失败时无声**。

| 编号 | 位置 | 证据 | 性质 |
|---|---|---|---|
| 一 | 用户列表搜索框 | `onSearch(_keyword: string)` 参数带下划线前缀=故意不用，旁边注释「搜索逻辑由具体业务实现」 | 控件是摆设 |
| 二 | 审计日志筛选栏 | 前端 `auditApi.list({...filters})` 传 `action`/`username`，后端 `PaginationParams` 只有 page/page_size/sort，`serde` 默认忽略未知 query | **静默**丢弃 |
| 三 | `BaseUpload` 演示页 | `frontend/src/views/demo/index.vue:62` 写死 `:action="'/api/upload'"`，全仓库 `grep -rn "api/upload" src/` **0 命中** | 指向不存在的端点 |

第三个尤其值得记：它是**演示页**，会被别人当模板抄走，抄一次就得到一个 404。
这与前两处同源——都是"控件存在 ⇒ 使用者以为能力存在"。

**这一族必须一起修**，且判据要落在**返回条数**上，不是"参数发出去了"。

## 真功能缺口（按价值排序，不是按工作量）

| 缺口 | 证据 | 为什么重要 |
|---|---|---|
| **登录/注册完全不落审计** | `/api/auth/login`、`/api/auth/register` 在 `public_routes`（router/mod.rs:135-138），**没有挂 `audit_log_middleware`**；失败只进 Redis 计数器，带 TTL 会过期 | 事后追不出"谁在何时从哪登录"。这是**安全追溯的基本盘**，不是锦上添花 |
| 审计日志导出**静默截断** | `demo.rs:174` 硬编码 `LIMIT 10000`，且 handler 签名**根本不接 Query**——导出永远无法带筛选，且超量被悄悄砍掉 | 用户以为导出了全量，实际只有最新 1 万条，且无任何提示 |
| 无自助改密 / 个人中心 | 无对应端点，改密只能管理员 `reset-password` | 用户被管理员重置才知道自己该改密码 |
| 角色列表无分页 | `list_roles` 无分页参数 | 小，随规模恶化 |
| 口令策略只有长度 | `validate_password` 仅 6–128，无复杂度/过期/历史/首次强制改密 | 与已有 Argon2 + 锁定机制不匹配 |
| 无数据范围过滤 | `UserRepository::list_all` 返回全量 | 属**产品决策**，不该我默认定 |
| 公开注册无审批/邮箱验证 | `POST /api/auth/register` 直接建号并授 `user` 角色 | 属**产品决策** |

## 明确**不建议**加的（"后台管理系统通常有"≠"这个项目该有"）

部门/组织架构、公告通知、用户头像上传、在线会话管理——
这些都是通用后台模板的标配，但**都依赖产品形态**（这是内部系统？多租户 SaaS？
有没有下游系统要对接用户？）。在不知道形态的情况下加它们，是拿工作量赌一个猜测。

例外：**在线会话管理**有一定独立性——现在只能登出当前令牌，
管理员无法在用户设备丢失时强制其下线。但这仍归 v0.12.0，与数据范围一起做，
因为两者都触及"身份"这个核心模型。

## 开发计划（重新排期）

### v0.10.0 — 停止说谎：三处假接口变真 + 日志可查

| 项 | 内容 | 判据 |
|---|---|---|
| 1 | 用户列表真筛选（keyword 匹配 username/email） | 传关键字后 `total` 变少 |
| 2 | 审计日志真筛选（username/action/status_code/时间范围） | 同上；且**未知参数不再静默丢弃**（`deny_unknown_fields` 或显式校验） |
| 3 | 日志导出支持筛选、去掉静默 `LIMIT` 或明示截断 | 导出条数与筛选条件一致；截断时响应里说清 |
| 4 | `BaseUpload` 接真后端 **或** 从演示页摘掉 | 二选一，不能留指向 404 的控件 |
| 5 | 角色列表分页 | — |

第 2 项里"未知参数不再静默丢弃"是这一版的**真正重点**：
只要 `serde` 继续默认忽略未知字段，将来新增任何筛选条件都会再次悄无声息地失效，
第 1、2 项修完也会复发。

### v0.11.0 — 登录可审计 + 自助改密

- 登录成功/失败/注册**全部落审计**（含 client_ip）。失败记录不能只靠 Redis 计数器
- 自助改密 + 个人中心：改密后**吊销存量会话**（与 `iat_ms` 精度对齐），
  且**不能**让用户改自己的 `is_active` 或角色
- 口令策略：复杂度下限 + 首次登录强制改密（注意：`iat_ms` 升级已让存量令牌作废一次，
  别再叠加第二次强制登出冲击）

### v0.12.0 — 身份模型（需产品先定形态）

- 在线会话管理：列出会话、强制下线单设备
- 数据范围 / 行级过滤（**策略由产品定**：按部门？按创建人？全量？）

### 待产品决策，仍不塞进任何版本

- 公开注册是否关闭（建议关，改管理员建号）
- 是否需要部门/组织架构、公告通知、头像上传（依赖产品形态）
- Prettier 进 CI（老问题）

## 跨版本纪律（从 v0.7.0 到 v0.9.0 沉淀，值得单列）

1. **能自动发现的清单不要人列**：写入口清单从 OpenAPI 派生；
   权限码清单从 `src/model/permission.rs` 派生；
   **下一步应把"前端发出的 query 参数"也纳入契约测试**——
   这正好能自动抓出缺口二
2. **同一件事的每个入口都要查**，且要问：**它等价于什么已有操作的镜像？**
   v0.7.0→v0.9.0 三个安全洞全是这个问题；本轮三处"说谎"是它的 UI 版本
3. **静默失败比报错更危险**：筛选被丢弃、导出被截断、控件指向 404——
   三者都不会报错，所以都不会被发现。**新增能力时优先让它"响着失败"**

---

# v0.10.0 — 停止说谎：三处假接口变真 + 日志可查

## 当前目标

把"界面提供了控件、后端却没有对应能力、且失败时无声"这一族缺口关掉。
与前三版同源（"授权的两面只装了一面"的 UI 版本），但危害不同：
安全洞是**放行了不该放行的**，假接口是**让人以为能力存在而其实没有**。

**这一版的真正重点不是把三个筛选做出来，是让"未知参数"不再静默丢弃**——
只要 `serde` 继续默认忽略未知字段，将来新增任何筛选条件都会再次悄无声息失效，
前两项修完也会复发。

## 当前计划

| 步骤 | 内容 | 状态 |
|---|---|---|
| 0 | 记录目标与起始 git 状态 | ✅ 已完成 |
| 1 | 用户列表真筛选（keyword → username/email） | ✅ 已完成 |
| 2 | 审计日志真筛选 + **未知参数不再静默丢弃** | ✅ 已完成 |
| 3 | 日志导出支持筛选 + 去掉静默 `LIMIT 10000` 或明示截断 | ✅ 已完成（含前端接线） |
| 4 | `BaseUpload` 接真后端**或**从演示页摘掉 | ✅ 已摘掉 |
| 5 | 角色列表分页 | ✅ 已完成（破坏性变更） |
| 6 | 契约测试：前端 query 参数集合 vs 后端 DTO 字段 | ✅ 已完成 |
| 7 | 集成测试 + 缺陷注入验证 | ✅ 已完成 |
| 8 | 质量门禁（含 e2e + 授权探针） | ✅ 全绿 |
| 9 | 文档：CHANGELOG / README / 版本号 | ✅ 已完成 |
| 10 | 提交（**不推送**） / tag / Release | 提交 ✅；tag 与 Release 待用户指令 |

## 起始 git 状态

- 分支 `master`，与 `origin/master` 同步（0 提交待推）
- HEAD = `b4cb627a docs(handoff): 记录 v0.9.0 后的功能缺口复查`
- 工作区：仅本文档改动
- 版本号：Rust / frontend 均 `0.9.0`；tag `v0.9.0` 已发布

## 关键决定（动手前先定，避免中途反复）

1. **未知参数的处理方式待验证后定**。候选：`deny_unknown_fields`（严格但会
   打断兼容性）vs 显式校验并报 400 vs 保持忽略但**加契约测试**。
   倾向"显式校验 + 契约测试双保险"，但要先确认前端有没有在传后端不认的参数
2. **导出的 `LIMIT 10000` 不静默保留**。要么支持筛选 + 大导出走异步，
   要么保留上限但**在响应里明示**"已截断"。静默砍数据比报错更糟
3. **`BaseUpload` 倾向摘掉而非接后端**。文件上传要牵出对象存储、病毒扫描、
   鉴权与配额，是独立议题；为一个演示页引入它不划算。但摘掉前要先确认
   `BaseUpload` 组件本身是否还有别的用处（目前只有演示页在用）
4. **筛选判据落在返回条数上**，不是"参数发出去了"。每条都要能证明过滤真的生效

---

## 步骤 1–3 完成记录（2026-10-02）

### 步骤 1：用户列表真筛选 ✅

- `keyword` 匹配 username/email，LIKE 前过 `escape_like_pattern`（`%`/`_` 当字面量）
- `UserListParams` 加 `deny_unknown_fields`
- 顺手修掉一个真问题：`Query<T>` 解析失败时 axum 默认回 `text/plain` 400，
  **绕过统一响应格式**。新增 `From<QueryRejection> for AppError`，
  user/demo/menu 三个 handler 改用 `Result<Query<T>, QueryRejection>`。
  handler 参数顺序保持 `State → 权限守卫 → Query`，不破坏"权限先于校验"的约定

### 步骤 2/3：审计日志筛选 + 导出截断明示 ✅

- `AuditLogQuery` 9 字段显式列全 + `deny_unknown_fields`
- `push_filters` 用 `QueryBuilder` 动态拼接。**不用** `$1 IS NULL OR ...` 恒真写法——
  那种写法优化器化不成索引扫描
- 导出 `fetch_for_export` 多取 1 行判截断，返回 `(rows, truncated)`，
  响应头 `x-export-row-count` / `x-export-truncated` / `x-export-max-rows`

### 测试过程中发现的两个坑（都不是实现错，是测试错）

1. **`GET /api/admin/users/{id}` 不存在**——该路由只注册了 PUT/DELETE，
   拿它造 404 日志实际拿到 405。改用 `DELETE` 不存在的用户 id（`find_by_id` 报 404）
2. **审计中间件是 `tokio::spawn` 异步落库**，"发完写请求就查"有竞态。
   沿用 `admin_requests_are_written_to_audit_log` 的轮询写法，
   新增 `wait_for_logs` helper。判据必须指向**本次测试自己造的那条日志**——
   用"items 非空"当判据会自证成功（查询自身的 GET 也会被记一条）

### 缺陷注入（两处，均已还原复验）

| 注入 | 预期 | 结果 |
|---|---|---|
| `push_filters` 直接 return | 筛选测试红 | ✅ 4 条全红 |
| 导出 handler 提前 return（不设响应头） | 导出测试红 | ✅ 红在"应回传 x-export-row-count" |

注入第一版时 `_by_action` / `_by_username` 仍是绿的——判据只验"目标行在里面"，
不筛选时全量也满足。已改为**双向收窄证明**：造一条**别的用户**的日志，
断言按该用户筛得到、按 admin 筛不到；`action` 用本次测试独有的 role_id 当筛选值。

---

## 下一步

1. 步骤 8：全量门禁（fmt / clippy / 单元 / 集成 / e2e 3 套件 / 授权探针 / 前端四项）
2. 步骤 9：CHANGELOG + README + 版本号 `0.9.0 → 0.10.0`
   （**必须写明角色列表的破坏性变更**）
3. 步骤 10：提交（**不推送**，等用户指令）

---

## 步骤 4–6 完成记录（2026-10-02）

### 步骤 4：`BaseUpload` 从演示页摘掉 ✅

- `frontend/src/views/demo/index.vue` 移除上传卡片，组件本身保留（`components/common/BaseUpload.vue`）
- 原代码写死 `:action="'/api/upload'"`，**该端点在 `src/router/mod.rs` 里根本不存在**
- 摘掉而非接后端：文件上传要牵出对象存储、病毒扫描、内容类型校验与配额，是独立议题

### 步骤 5：角色列表分页 ✅（**破坏性 API 变更**）

- `role_repo::list_all()` → `list_paginated(page, page_size)`，返回 `(当页, 总数)`
- `GET /api/admin/roles` 返回值 `Vec<RoleItem>` → `PaginatedResponse<RoleItem>`
- **`ORDER BY r.created_at ASC, r.id ASC`**：加了 `r.id` 作并列时的 tiebreaker。
  只按 `created_at` 排序会在时间戳相同时让翻页出现重复/漏行
- 连带修两处会被打破的消费方：`tests/api_integration.rs`、`e2e/probe-write-guards.mjs`

**分页引入的新坑（已堵）**：用户表单的角色下拉走同一个接口，
默认 `page_size=10` 会**只显示前 10 个角色**——用户看不到自己实际持有的角色，
保存时可能把权限改掉。加 `roleApi.listAll()` 按 200/页 翻页取全，
且总数对不上时抛错而非静默截断。配套 3 条单测。

### 步骤 6：契约测试 ✅

两条纯静态测试（不需要 DB，`cargo test` 直接跑）：

1. `frontend_query_params_match_backend_dto_fields`
   —— 5 组前后端字段对照（user / role / audit / menu / dict），**双向**报：
   前端发了后端不认识的字段（危险方向），以及后端有、前端从不发的字段（入口没接上）
2. `every_query_dto_rejects_unknown_fields`
   —— 扫描 `src/controller/*.rs` 里所有 `*Query` / `*Params` 结构体，
   要求紧邻上一行是 `#[serde(deny_unknown_fields)]`。**不靠人列清单**，
   新增 DTO 自动纳入检查

顺带补齐：`MenuQuery`、`DictItemQuery` 此前**没有** `deny_unknown_fields`，
且 dict 的 `list_items` 还在走绕过统一响应格式的旧写法，都已修。

### 步骤 4–6 的缺陷注入（均已还原复验）

| 注入 | 结果 |
|---|---|
| `push_filters` 直接 return | ✅ 4 条筛选测试全红 |
| 导出 handler 提前 return（不设响应头） | ✅ 红在"应回传 x-export-row-count" |
| 角色 handler 忽略 `page`（写死 1） | ✅ `role_list_pages_over_the_same_set…` 红 |
| 前端 `menu.ts` 多发 `parentId` | ✅ 报"发出后端 MenuQuery 不认识的字段" |
| 摘掉 `RoleListParams` 的 `deny_unknown_fields` | ✅ 报"紧邻的上一行是 `#[derive(…)]`" |

### 踩到的第三个坑：测试自己也会说谎

`updating_a_role_returns_the_real_row` 在步骤 5 之后变红——它请求
`GET /api/admin/roles` 不带分页参数，默认 `page_size=10`，
而排序是 `created_at ASC`，前面用例留下的角色已把新建的挤到第 2 页之后。
**不是分页的 bug，是测试假设了"列表=全量"**。已显式改成 `page_size=200`。

这和 v0.9.0 那次「失败的运行 panic 跳过清理留下脏数据」同源：
测试库长期存在，任何依赖"库里现在恰好有多少数据"的断言都会随执行顺序漂移。
**断言要写性质，不写全局计数。**

---

## 步骤 7–9 完成记录（2026-10-02）

### 步骤 7：集成测试 + 缺陷注入验证 ✅

新增测试覆盖四条筛选路径（用户 keyword、审计五条件、导出复用同一套条件、角色分页），
并为每条配了**缺陷注入**：把实现改坏，确认对应测试变红，再还原复验。

| 注入 | 结果 |
|---|---|
| `push_filters` 直接 return | ✅ 4 条筛选测试全红 |
| 导出 handler 提前 return（不设响应头） | ✅ 红在"应回传 x-export-row-count" |
| 角色 handler 忽略 `page`（写死 1） | ✅ 分页测试红 |
| 前端 `menu.ts` 多发 `parentId` | ✅ 契约测试报"发出后端 MenuQuery 不认识的字段" |
| 摘掉 `RoleListParams` 的 `deny_unknown_fields` | ✅ 报"紧邻的上一行是 `#[derive(…)]`" |

### 步骤 8：全量门禁 ✅

| 门禁 | 结果 |
|---|---|
| `cargo fmt --check` | ✅ |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | ✅ |
| 单元测试 | ✅ 58 passed |
| 集成测试 | ✅ 88 passed（`--ignored`）+ 5 passed（非 ignored） |
| e2e | ✅ 4/4 套件（新增 `v010-ui-truth.mjs`，13 条断言） |
| 授权探针 | ✅ 41/41 |
| 前端 lint / typecheck / test / build | ✅（1 条 `env.d.ts` 既有 warning） |

行为探针确认后端连的是测试库（`roles total=34`），不是误连开发库。

### 步骤 9：文档与版本号 ✅

- 版本号三处同步到 `0.10.0`：`Cargo.toml` / `Cargo.lock` / `frontend/package.json`
- `CHANGELOG.md` 新增 0.10.0 段落；`README.md` 同步筛选能力与分页 breaking change
- 三份中文文档写入后 `grep -c $'\ufffd'` 均为 0，无乱码

### 步骤 10：提交 ✅（**未推送**）

- 本次提交即步骤 10 的"合并到 master"；**tag 与 Release 等用户指令再打**
- 用户指令：本地跑门禁 + 提交，**全部工作完成或收到指令后统一推送**

---

## 本版跨版本沉淀：为什么"契约测试"比"修 bug"更值钱

v0.10.0 修的三个 bug 都不是复杂逻辑错误，**是"加了参数但没接到后端"**。
这类 bug 修完还会复发，所以真正的产出是那条契约测试：

- 前端发了后端不认识的字段 → 立即报（危险方向：拼错参数静默失效）
- 后端有字段前端从不发 → 立即报（入口没接上：筛选条件形同虚设）
- 新增 DTO 未加 `deny_unknown_fields` → 立即报（不靠人列清单）

前两条管**当下**，第三条管**将来**。只要 `serde` 默认忽略未知字段的默认行为还在，
契约测试就是唯一能挡住复发的东西。

### 与 `deny_unknown_fields` 相关的设计约束

**`deny_unknown_fields` 与 `serde(flatten)` 不兼容**。因此查询结构体
**必须显式列全字段**，不能靠 flatten 收敛公共参数（如分页）。
这是有意接受的代价：显式列举让"这个接口到底认哪些参数"变成可 grep 的事实。

**`push_filters` 用 `QueryBuilder` 动态拼接**，不是 `$1 IS NULL OR $2 IS NULL`。
后者语义上等价，但会把计划固定成无法走索引的形态，数据量上来后是隐性退化。

**断言写性质，不写全局计数**。测试库长期存在，任何"库里现在恰好有 N 条"的断言
都会随执行顺序漂移。角色列表分页后暴露的 `updating_a_role_returns_the_real_row`
就是这么变红的——不是分页有 bug，是测试假设了"列表=全量"。

### 三个只有真跑起来才会遇到的坑

1. **审计中间件是 `tokio::spawn` 异步落库**。"发完写请求立刻查"有竞态，
   必须轮询等待，且判据要指向**本次测试自己造的那条日志**，
   否则会命中别的测试留下的历史数据而假绿
2. **`GET /api/admin/users/{id}` 根本不存在**（只有 PUT/DELETE）。
   拿它造 404 实际拿到的是 405，测试会因错误的原因变绿
3. **审计的 `action` 字段是 `"{METHOD} {path}"`**，不是裸路径。
   按裸路径断言会永远匹配不上

另有前端侧的坑：**GET 缓存命中时返回伪造 response**（`headers` 只有
`{'x-cache': 'HIT'}`），会吞掉 `x-export-truncated`。二进制响应已排除出缓存。

