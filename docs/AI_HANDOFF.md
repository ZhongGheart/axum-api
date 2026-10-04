# AI_HANDOFF — 崩溃恢复日志

本文件是跨会话的交接日志。任何非平凡改动在**动手前**先写这里，达成里程碑后更新。
接手者必须先把它与 `git status` / `git diff` / 实际文件系统对账。

## v0.22.0 系统参数配置表 + 口令策略（2026-10-04 会话 · ✅ 已完成，未推送）

### 当前目标

用户指令：**执行 v0.22.0**。

### 起始 git 状态

- 分支 `master`，工作区干净，`HEAD == ded82c37`，与 origin/master 同步（v0.21.0 已发布）
- 后端 `127.0.0.1:8080`；前端 vite `localhost:3000`；PG 55432 / Redis 56379

### 范围取舍：只做 C2 + C3，C1 推到 v0.23.0

ROADMAP 的 v0.22.0 表格列了 C1/C2/C3/C4 四项，但**它自己的排序理由否掉了其中两项**：

- 「C1 独立但量大，可以和 C2 并行，但**不要同版**——树形递归删除的边界情况
  （父子循环、跨部门角色授权）需要独立的测试预算」
- 「C4 ... 建议单独占一版而不是塞进 v0.22.0」

所以本版 = **C2（系统参数配置表）+ C3（口令复杂度与过期策略）**。
C3 硬依赖 C2（策略要可配），两者同版是自洽的；C1 顺延到 v0.23.0。
这是按路线图**最经得起推敲的那句**执行，不是自行缩小范围。

### 计划

1. 迁移 `016_system_settings.sql`：参数表 + 存量口令时间戳列
2. `model/setting.rs`：参数定义的**单一数据源**（照抄 `permission.rs` 的 const 表模式）
3. `repository/setting.rs`：读写 + Redis 缓存与失效
4. `service/setting.rs`：类型化读取、范围校验
5. `controller/setting.rs`：管理端 CRUD + 面向登录页的公开读取
6. `utils/validation.rs`：口令策略改为**由参数驱动**，不再是写死常量
7. 前端：系统参数页 + 登录/注册页实时策略提示 + 口令强度指示
8. 测试：参数契约、缓存失效、口令策略边界、**缺陷注入**

### 硬约束（沿用）

- 口令策略**绝不能**进入登录校验路径（`password_policy_is_not_applied_to_login_verification` 已钉住）
- 抬高口令强度门槛的当天，**存量弱口令用户不能被锁在门外**
- 每个修复都要做缺陷注入验证

### 环境注意

必须带 `RATE_LIMIT_IP_MAX=100000` / `RATE_LIMIT_USER_MAX=100000` 起后端，否则整轮 e2e 因限流假红。

### 里程碑：后端完成（迁移 016 + 参数表 + 口令策略接线）

后端 7 个新文件 + 16 个改动文件已落地，`cargo test --lib` 108 passed。
新增 13 条集成测试逐条单跑均通过。

**本轮踩到并已修掉的坑（写下来是因为它会再犯）**：

1. **参数表种子值静默盖掉了部署配置。** 首次集成时 `LOGIN_MAX_FAILURES=3`
   被种子值 10 盖掉，于是「失败 3 次应锁定」变成 200，**日志里一个字都没有**。
   修法确立为：判定「管理员显式改过」靠 `updated_by IS NOT NULL`，
   改过 → 参数表；没改 → 环境变量。判定由数据本身承载，不依赖内存状态。
2. **存量库拿不到新权限码。** `seed_menus_if_empty` 只在 `menus` 表为空时写整棵树，
   而已存在的部署 `menus` 非空，于是 `/system/setting` 永远种不进去；
   权限码按 `parent_path` 解析父菜单，解析不到就**只打一行 warn 然后跳过**——
   管理员手动授权也授权不了。新增 `backfill_late_added_menus` 补齐。
3. **两条测试自身有缺陷**（不是产品缺陷，但会让整轮结果不可信）：
   `put_setting_raw(...)` 漏 `.await` 导致清理不执行、状态串味；
   `the_settings_endpoints_require_their_permission_codes` 撤销 admin 授权后
   未恢复，污染了后续两条测试。均已修（加了 `grant_permission_code()` helper）。

### 里程碑：前端完成（参数页 + 策略驱动的口令校验）

新增 `frontend/src/api/setting.ts`（5 个接口）、`frontend/src/stores/setting.ts`、
`frontend/src/views/system/setting/index.vue`。

**本轮最重要的前端决定：口令策略从常量变成运行时取值。**

管理员能在参数页改最小长度/类别数/大小写混合，而前端原先把 8 位 / 两类
写死在 `utils/password.ts` 与 `accountRules.ts` 里。若不改，后果是
**界面提示的规则与后端实际执行的规则分叉**：用户按提示填一个合规口令、
提交后被拒，而报错在说另一件事——界面在主动误导人。

改法：
- `passwordIssues(password, policy)` 吃传入策略；省略参数时用
  `DEFAULT_PASSWORD_POLICY`（与后端 `PasswordPolicy::default()` 逐字一致）
- `passwordPolicyRules` / `PASSWORD_PLACEHOLDER` / `PASSWORD_MAX_LEN`
  **从常量变成函数**，由注册页、改密页、管理员建号三处传入 store 里的策略
- 判定失败原因（`passwordIssues`）与界面要求提示（`describePasswordPolicy`）
  **共用同一个 `classRequirementText`**——两处各写一份时漏改的那处会开始说另一件事
- `stores/setting` 的降级路径：取不到策略时保留回落值且**不抛、不弹错**。
  它是纯提示性增强，为它弹红字会让「服务端暂时不可达」看起来像「注册坏了」

前端门禁全绿：`typecheck` ✓、`lint` 0 error（余 1 个 env.d.ts 既有 warning）、
`test` 203 passed（21 个文件）。后端 4 条前后端契约测试重跑仍绿。

### 里程碑：缺陷注入验证完成（5 处，1 处暴露了真实测试缺口）

| 注入 | 结果 |
|---|---|
| 删掉注册时的 `password_changed_at` 写入 | **首次全绿 —— 暴露真实缺口** |
| 口令过期改成拒绝登录 | ✅ 红 |
| 参数表无条件优先于部署配置 | ✅ 红 |
| 去掉 `backfill_late_added_menus` | ✅ 红 |
| `require_mixed_case` 只改文案不改判定 | ✅ 红 |

**第一条是本轮最有价值的产出。** 注入后
`an_expired_password_yields_a_restricted_token_...` **依然通过**，
因为它自己用 `UPDATE` 把时间戳改成 100 天前——它验的是**判定逻辑**，
从不验**写入路径有没有写**。而 `is_expired(None)` 按设计判为未过期，
于是这个漏表现为**过期策略对每一个新用户永久静默失效**：
参数页显示着 90 天，界面无任何异常，只有安全策略不在了。

补了 `every_path_that_sets_a_new_password_records_when_it_was_set`
（注册 / 自助改密 / 管理员重置三条路径），重跑注入即红。
**教训：一条测试只能钉住它设计时要验的那一层。**

写这条测试时踩了两个自己的坑：路由是 `POST /api/admin/users/{id}/reset-password`
（不是 `PUT .../password`），请求体字段是 `password`（不是 `new_password`）——
第一次写成 404 才改对。

**注入型红测会留脏数据**（中途 FAILED 的用例不会执行清理）：
`expiry_days` 被留在 30、`require_mixed_case` 被留在 true、
`mixedon_*` / `expiring_*` / `stamped_*` 账号残留。收尾必须查一遍
`system_settings` 的 `updated_by` 与测试账号前缀。

### 状态：v0.22.0 已完成，本地提交，**未推送**

门禁全绿：

| 项 | 结果 |
|---|---|
| `cargo fmt --check` | ✓ |
| `cargo clippy --all-targets -- -D warnings` | ✓ 0 warning |
| `cargo test --lib` | 108 passed |
| `cargo test --test api_integration -- --ignored --test-threads=1` | **176 passed / 0 failed** |
| `pnpm lint` | 0 error（余 1 个 env.d.ts 既有 warning） |
| `pnpm typecheck` | ✓ |
| `pnpm test` | 203 passed / 21 文件 |
| `pnpm build` | ✓ |

文档已更新：CHANGELOG 0.22.0 条目、README（API 表 / 权限码表 / 环境变量降级说明 /
「从 v0.21 升级到 v0.22」）、ROADMAP（v0.22.0 标记完成，C1 顺延 v0.23.0 并写明取舍）。
版号：Cargo.toml + frontend/package.json 均抬到 `0.22.0`。

**按用户既定规矩：本地提交，不推送。** 等明确指令再推 + 打 tag。

### 下一版（v0.23.0）建议顺序

ROADMAP 已重排为「组织结构 + 规模化」，并把 C1 部门树从 v0.22.0 移进来了。
若只做一项：**D4 审计日志导出 / 更细检索**（后端明细能力已写好，只差端点 + 前端检索页，
性价比最高）。若做两项：**D4 + C1**（C1 需独立测试预算，不要与其他功能混）。

---

## v0.21.0 UI 梳理与优化（2026-10-04 会话 · 当前）

### 当前目标

用户指令：**全面梳理一下系统的 UI，优化一下界面交互及配色**。纯前端改造，未动后端 Rust。

### 状态：已完成并发布 —— v0.21.0 已推送、已打 annotated tag

### 起始 git 状态

- 分支 `master`，工作区干净，`HEAD == 596ee068`，ahead origin/master 4 个提交
  （本轮结束时已 ahead 6 个提交并全部推送，打 tag `v0.21.0`）
- 后端 `127.0.0.1:8080`；前端 vite `localhost:3000`；PG 55432 / Redis 56379

### 顺手修掉的两个真缺陷（视觉验证才发现，代码读不出来）

**1. 首页路由从来没渲染过**（`/`）

布局壳 `Root` 的 `path` 是 `/`，而菜单种子数据里首页也是 `path='/'` +
`component='home/index'`。`buildRoutesFromMenus` 把它当成**绝对子路径**注册，
于是 vue-router 里出现一条与父级同路径的子记录：父级先命中、子级永不命中。
表现是**登录后首页一片空白**（router-view 只渲染出 `<!---->`），
而侧栏菜单、面包屑、`/system/user` 等全部正常——所以一直没人发现，
`views/home/index.vue` 那句"后续替换为 Dashboard"的占位卡片其实**一次都没显示过**。

修法：`menuRoutes.ts` 跳过 `path === '/'`，改由 `router/index.ts` 里 Root 的
**空路径子路由**静态承载首页。侧栏入口仍由后端菜单驱动，指向同一个 `/`。

> 判定手法：`git stash` 后看原代码下 `.layout-content` 的 DOM，
> 确认原代码同样是空的 —— 排除了"是我改坏的"。选择器要用 `.layout-content`
> 作用域，`document.querySelector('.n-scrollbar-content')` 会先命中**侧栏**的滚动条。

**2. 侧栏菜单显示函数源码**

`renderMenuLabel` 写成了"返回一个函数"，而 naive-ui 要求它**本身就是**渲染器。
naive 把那个函数当成待渲染内容，侧栏于是显示
`() => appStore.collapsed ? h("span", ...) : option.label`。

### 本轮改动

| 类别 | 内容 |
|---|---|
| 配色 | `stores/app.ts` 收敛主色为偏青靛蓝 `#2b5fd9`（避开 naive 默认蓝 + 登录页紫蓝打架），补 `Card/DataTable/Button/Menu/Input/InternalSelection/Layout` **组件级** theme-overrides；暗色 `placeholderColor` 由 `#666` 提到 `#848b98`（原值对比度约 3.4:1） |
| 设计令牌 | `global.css` 补齐表面/文本/状态/边框/圆角语义变量、`focus-visible`、`prefers-reduced-motion`；`.page-container` 与 Card 的圆角边框统一 |
| 布局 | Logo 由 emoji ⚡ 换成图标；顶栏用上 v0.20.0 的 `avatar_url`（此前硬编码蓝底首字母）；面包屑按菜单树回溯祖先链补全层级；折叠/主题按钮加 tooltip；折叠态菜单项补 `title`；`.user-info:hover` 由写死的 `rgba(0,0,0,.05)` 改走变量（暗色下原本几乎不可见） |
| 菜单图标 | `MENU_ICONS` 扩到 20+ 键；新增 `PATH_ICONS` **按路径兜底**——后端给 6 个菜单都种了 `settings`，侧栏曾有六排一模一样的齿轮。改的是显示，不动菜单表的 `icon` 字段（那是管理员的数据） |
| 首页 | 占位页 → 真实仪表盘：4 个指标卡 + 最近操作 + 运行状态 + 刷新按钮。**按 `permissionsStore` 逐项降级**：没权限的指标**不发请求也不渲染**（不是渲染成 0 或报错），全无权限时给单个空状态 |
| 认证页 | 新增 `AuthShell.vue` 供登录/注册共用，抽掉两份重复的紫渐变；标签从左侧固定宽改顶部；去掉 `letter-spacing: 4px` 与"登 录/注 册"加空格；标签文案由"记住密码"改为**"记住用户名"**（实现本来就只记用户名） |
| 数据展示 | 新增 `utils/time.ts` 统一时间格式化；用户页时间列原先直出 RFC3339 且在列宽不足时折行；角色页原先手写 `.replace('T',' ').slice(0,19)`，**带时区偏移时会算错一个时区且看起来完全正常**；日志页同样处理 |
| 头像 | 抽出 `utils/avatar.ts`，顶栏/个人中心/用户列表三处共用；用户列表此前 `n-image` + 空 `fallbackSrc` 在加载失败时**画出碎图图标**，28px 的碎图比留白更像故障，改为回退首字母圆形 |
| emoji | 监控页/仪表盘的 🟢🔴 状态 emoji 换成 CSS 圆点（emoji 在不同系统上字形与基线都不一致） |

新增文件：`utils/avatar.ts`、`utils/time.ts`、`components/common/AuthShell.vue`、
`utils/__tests__/{avatar,time}.spec.ts`

### 门禁结果（全绿）

| 项 | 结果 |
|---|---|
| `pnpm typecheck`（vue-tsc） | ✅ 0 error |
| `pnpm lint` | ✅ 0 errors（`env.d.ts:5` 1 个历史 warning） |
| `pnpm test` | ✅ **180 passed**（原 161，新增 19） |
| `pnpm build` | ✅ built in 16.12s |

### 视觉验证（Chrome 扩展报 "Codex auth token is unavailable"，改用 Codex 内置浏览器）

登录页 / 首页亮色 / 首页暗色 / 系统监控 / 用户表格 / 普通用户首页，逐一截图核对；
控制台无 error。**普通用户（`user` 角色）实测零 403**——仪表盘不发它无权请求的接口。

> 验证用的 `uidemo` 测试账号已删除，演示库恢复为 2 个用户。

### 踩过的坑（留给下一个人）

- **`.vue` 导入必须带显式后缀**：`@/components/common/AuthShell` 解析不到，
  `@/components/common/AuthShell.vue` 才行。项目里所有 `.vue` 导入都带后缀。
- **`vue` 文件里有嵌套 `<template>`（slot）时**，`s.index('</template>')`
  会截到**内层**插槽，留下游离的旧模板尾部。Vue 不报错、tsc 也不报错，
  但页面渲染成空白。**必须用 `rindex`**。这个坑在 `login/index.vue` 和
  `MainLayout.vue` 各踩了一次。
- 中文写入必须 `python3` + `io.open(encoding='utf-8')`；本轮又踩了一次替换字符
  （`docs/AI_HANDOFF.md`、`utils/avatar.ts`、`router/index.ts` 各一次），写完要 grep 查。
- 改完颜色要去 grep 硬编码：`#2080f0` / `#d03050` / `letter-spacing` /
  `color: #888` 之类，散落在 `BaseUpload` / `demo` / `NotFound` / `monitor` 等处。
- 仪表盘的跳转路径**不能写死**：真实路径是 `/system/monitor/system`（不是
  `/monitor/system`），而且菜单路径是管理员可改的数据。已改为按 component
  标识从菜单树反查，查不到就不渲染按钮。

### 后续可做（未做，供下一轮取舍）

- `BaseTable` 与 `EnhancedBaseTable` 功能大量重叠，且**只有 user 页与 demo 页在用**，
  其余页面各自裸写 `n-data-table`。要统一交互得先决定这两个组件的取舍。
- `SearchForm` 没有展开/收起，筛选项多时会撑满一行。
- 各页面空态/错误态仍未统一（仪表盘与登录/注册已统一）。
- 表格密度、列宽、批量选择交互未系统梳理。

### 约束提醒

- 用户规矩：默认**本地提交不推送**，等明确指令才推；本轮用户明确说了"推送打版"，故已推
- **版本号占用要同步路线图**：v0.21.0 原被 ROADMAP 的"组织与配置"预留，
  本轮 UI 改造占了同一号，已把 C/D 两线顺延为 v0.22.0 / v0.23.0 并在文首写明原因。
  不改的话同一个版本号会被两处同时声称
- 视觉验证默认 **Google Chrome**；本轮 Chrome 扩展不可用，改用 Codex 内置浏览器

## v0.20.0 规划（2026-10-03 会话 · 当前）

### 当前目标

**只做规划，不写实现代码。** 交付物是 `docs/ROADMAP.md`（v0.20.0 / v0.21.0 / v0.22.0 三段路线）。

v0.19.0 已推送，但**发布元数据是缺的**——版号仍是 `0.18.0`，没有 v0.19.0 的 tag、
CHANGELOG 条目或 Release。用户当时只说"推送"没说"发布"，所以没动。
**v0.20.0 开工前要先决定**：是补齐 v0.19.0 的发布元数据，还是在 v0.20.0 里一并抬到 0.20.0。
这不是小事——tag 缺失会让 `git describe` 失效，CI 的版本注入也会继续报 0.18.0。

### 当前计划

| 步骤 | 内容 | 状态 |
|---|---|---|
| 0 | 接手 v0.19.0 推送后的会话，对账 git / 交接 / 文件系统 | ✅ |
| 1 | 复核缺口证据（跳过 v0.19.0 已排查项） | ✅ |
| 2 | 写 `docs/ROADMAP.md` 三段路线 + 每条线的风险与依赖 | ✅ |
| 3 | 在 `NEXT_VERSION_SCOPE.md` 顶部加失效指引（保留 v0.2.0 历史，不删） | ✅ |
| 4 | **等指令后才动手写实现** | ⏸ |

### 起始 git 状态

- 分支 `master`，工作区干净，`HEAD == origin/master == 7f4d5dbb`
- tag 最高是 `v0.16.0`（v0.9.0–v0.16.0 有 tag，v0.17.0 起没有）
- `Cargo.toml` / `frontend/package.json` 都是 `0.18.0`
- 规模：src 12,221 行 / frontend 10,417 行 / 集成测试 227 个函数

### 本轮复核过的证据（可直接引用，别重排）

| 结论 | 证据 |
|---|---|
| 用户**不能自助改资料** | `/api/auth/*` 只有 `password` 是 PUT（router/mod.rs:152），**无 profile 端点**；`profile/index.vue` 只有三个改密字段；`users` 表无 nickname/avatar/display_name |
| 文件上传是**悬空 affordance** | 前端 `BaseUpload.vue` 完整可用且已导出（components/common/index.ts:11），但**无人使用**；后端 `axum` 只开 `features=["macros"]` **没开 multipart**，无上传端点，`tower-http` 只开 cors/trace/set-header **没开 fs**；`demo/index.vue:60` 自己写着"BaseUpload 暂时不在这里演示" |
| **无在线会话登记** | 登录成功后**不写**任何会话记录，jti 只在登出时进黑名单（`service/auth.rs:366`）。Redis 有 `revoke_user_sessions`（按 user 整体吊销）与 `delete_by_prefix`，但没有"列出谁在线"的数据 |
| **锁定后管理员无法解锁** | `clear_login_failures(scope)` 唯一调用点在登录成功分支（`service/auth.rs:331`），无 admin 解锁入口。锁 TTL = `LOGIN_FAILURE_WINDOW` 默认 300s，自过期 |
| 用户列表筛选维度窄 | `list_users` 只收 `page/page_size/keyword`，keyword 同时匹配 username+email（`controller/user.rs` utoipa 参数表）。无按角色/状态筛选 |
| 权限码需新增 | `model/permission.rs` 29 个 const，用户域只有 `system:user:{list,create,update,delete}`。解锁/会话管理都要新码，且会被 `every_admin_handler_declares_a_permission_guard` 自动纳入守卫检查 |

新增 admin handler 会被 `every_admin_handler_declares_a_permission_guard`（tests/api_integration.rs:2657）自动覆盖；
新端点会被 `every_documented_endpoint_is_reachable_without_a_server_error`（从 openapi 派生 50 端点）自动纳入。
**写新端点时不要绕过这两条测试。**

### 本轮确认**不是**缺口（v0.19.0 已闭环，别再写进计划）

前端 GET 缓存无用户维度（`requestCache.invalidate()` 在登录成功 / 401 / 任何非 GET 后都调了）；
审计只记写操作（middleware 对所有方法都写）；"记住密码"存明文（只存 `{ username }`）。

### 里程碑：M0–M2 完成（发布元数据 / 迁移 014 / 自助资料 + 列表筛选）

**M0**：`CHANGELOG.md` 补 v0.19.0 完整条目（基于三个提交的真实内容）+ v0.20.0 占位；
`Cargo.toml` 与 `frontend/package.json` 抬到 `0.20.0`。版本号仍**未推送、未打 tag**。

**M1**：迁移 `014_user_profile_fields.sql` 加 `display_name VARCHAR(50)` / `avatar_url VARCHAR(512)`，
各带 CHECK。`USER_COLUMNS` 一并加两列（10 处手写列名的集中常量，改一处即全生效）。

#### 🐛 我自己写的迁移里有个真漏洞，是靠实跑才发现的

初版约束只写了：
```sql
CHECK (avatar_url IS NULL OR avatar_url ~ '^/uploads/[A-Za-z0-9._/-]+$')
```
而该字符类里**同时有 `.` 和 `/`**，于是 `..` 天然合法。实测：

```
UPDATE users SET avatar_url='/uploads/../etc/passwd';  →  UPDATE 1   ← 被接受了
```

应用层 `normalize_avatar_url` 有逐段 `..` 检查，所以 HTTP 路径拦得住；
但**绕过应用直写库**的路径会穿透，而这一列的 CHECK 存在的意义恰恰就是那条路径。
把它当"第二道防线"写在注释里，实际在穿越这一项上是空的。

修法是**加一条独立约束**而不是改正则——改正则会连带误伤 `a..b.png` 这类合法文件名：

```sql
CHECK (avatar_url IS NULL OR avatar_url !~ '(^|/)\.\.(/|$)')
```

`(^|/)` 与 `(/|$)` 保证 `..` 必须是**完整路径段**，否则 `..foo`、`a..b` 会被误拒。
复验（重建二进制后在新库上跑）：4 个穿越用例
（`/uploads/../etc/passwd`、`/uploads/a/../../b`、`/uploads/a/..`、`/uploads/..`）全部被拒，
而 `/uploads/a..b.png` 仍然接受。

#### 第二次栽在同一个坑：验证时用了没重编译的二进制

改完 014 后我建了新库重跑，**穿越用例仍然全部通过（UPDATE 1）**——差点以为修复无效。
真正原因是 `sqlx::migrate!` 在**编译期**把 SQL 嵌入二进制，我改了 `.sql` 文件但没 `cargo build`，
跑的还是旧迁移。**迁移文件改完必须重编译再验证**，否则看到的现象会把你引向错误的结论。

#### 展示名长度口径

50 个汉字存入成功（`char_length` = 50），51 个被 `varchar(50)` 拒。
应用层 `normalize_display_name` 按 `chars().count()` 判，先于 DB 拒绝，所以 HTTP 路径得到 400 而非 500。
这里刻意**不加**长度 CHECK：varchar 已经拒了，再加一条只会让报错从"字段超长"变成"约束冲突"。

**M2**：`PUT /api/auth/profile`（controller/auth.rs）+ `user_repo.update_profile` +
`auth_service.update_profile`；`list_users` 支持 `is_active` / `role` 两个新筛选维度，
`list_filtered` 从 `match keyword` 二分支改为**动态拼 WHERE + 顺序绑定**。

#### 两个设计决定（都不是随手写的）

**字段级三态 `Option<Option<String>>`**：外层区分"字段在不在请求里"，内层区分"要不要清空"。
`Option<String>` 下"不带字段"和`"display_name": null` 都是 `None`——
前端只想改头像时会顺手把展示名也清了，这是一次静默的数据丢失。
仓储签名因此是 `update_profile(id, Option<Option<&str>>, Option<Option<&str>>)`，
用 `COALESCE` 保持"缺省即不改"，前端不必先读旧值再原样写回（回写一个刚被别人改过的旧值
就是典型的丢失更新）。

**刻意不在受限令牌白名单里**：`middleware/auth.rs` 的 `pwd_stale` 分支只放行
password / logout / me。待改密的用户不能改资料——那个会话尚未确认凭据。

**列表筛选用 EXISTS 不用 JOIN**：一个用户可能同时命中多条角色行（多对多），
JOIN 会让同一用户重复出现并把 total 算大。`role_name` 与 `is_active` 同时给是 **AND**
（"既是 HR 又是禁用的"是一个明确的问法，改成 OR 会返回一批用户没预期的账号）。

**自查清理**：scratch 库 `axum_api_scratch` 与其中的 `probe` 用户是我这轮造的，
收尾要连库一起删。演示库 `axum_api_manual` 的 admin 行被我写测试用 SQL 动过
（avatar_url / display_name），需确认已复原。

### 开工记录：用户指令「执行新功能开发计划」

用户未反对上轮建议，按**建议方案**执行：(1) 发布元数据补齐并把版号一并抬到 0.20.0；
(2) 头像走本地磁盘 + `tower-http` ServeDir，对象存储抽象推 v0.22.0。

**将要改什么**：M0 发布元数据 → M1 迁移 014（`users` 加 `display_name`/`avatar_url`）→
M2 profile 端点 + 列表按角色/状态筛选 → M3 管理员解锁 + `system:user:unlock` →
M4 会话登记（登录写 Redis）+ 列举/单吊销 + `system:session:manage` →
M5 头像上传（开 axum multipart + tower-http fs + Docker 挂卷）→ M6 前端串联与门禁。
B3 CSV 批量导入插入 M5 之后。

**受影响文件**：`migrations/014_*.sql`（新）、`src/model/user.rs`、`src/model/permission.rs`、
`src/repository/user.rs`、`src/controller/{auth,user}.rs`、`src/router/mod.rs`、
`src/service/auth.rs`、`src/utils/redis.rs`、`src/config/mod.rs`、`Cargo.toml`、`Dockerfile`、
`frontend/src/views/profile/index.vue`、`frontend/src/views/system/user/index.vue`、
`frontend/src/api/*`、`tests/api_integration.rs`、CHANGELOG、README、Cargo.toml 与 package.json 版号。

**预期下一步**：M0+M1 落地并跑迁移，确认 `users` 新列落地后再动 M2 端点。

**每个里程碑都要做缺陷注入**（注入后必须红 + 回滚 + 查库清残留），这是本仓已确认的纪律。

### 里程碑：M3–M6 完成 + 一个把 53 条测试一次性打红的夹具缺陷

#### M3 解锁 / M4 会话 / M5 头像 / B3 CSV 的落点

- **M3** `POST /api/admin/users/{id}/unlock` + `system:user:unlock`。解锁复用登录的
  同一个归一函数算 scope（`account:{username}` / `account:{email}`），**故意不碰 IP 桶**——
  那是跨账号共享的，清了等于给爆破地址发新额度。
- **M4** 登录成功时登记 `sess:*`（靠 JWT 自身 TTL 自过期）；`GET /api/admin/users/{id}/sessions`
  返回**数组本身**（不是 `{items,total}`），`POST .../sessions/{jti}/revoke` 走 jti 黑名单。
  **登记失败不放行登录**：登记只用于管理视图，缺一条不影响认证结论，但会让"这个人
  在哪些设备登录"漏掉一次登录，而管理员正是靠这个列表判断账号是否被盗用。
- **M5** `POST /api/auth/profile/avatar`（multipart，字段名 `file`）。文件名由**服务端**
  生成 UUID，扩展名由 MIME 白名单推导（绝不用原始文件名拼路径）；
  `delete_uploaded_avatar` 校验 `uploads/avatars/` 前缀，防止任意文件删除；
  换头像时删旧文件，而**旧头像必须在写新值之前读出**。
  `/uploads` 的 `ServeDir` 刻意挂在鉴权之外（图片是 `<img src>`，带不了 Authorization 头）。
- **B3** `POST /api/admin/users/import`，请求 `{csv, dry_run}`。**逐行成败**，失败带行号；
  授权下界整批前置校验；口令不入审计。

#### 🐛 审计 `action` 列宽 100，超长路径的审计**整条静默消失**

`audit_logs.action` 是 `VARCHAR(100)`，而 `path` 是 `VARCHAR(500)`——
两列装的是同一段信息（`action = "{method} {path}"`），宽度却差 5 倍。
`POST /api/admin/users/{id}/sessions/{jti}/revoke` 拼出来是 **107 字符**，
INSERT 直接失败；而中间件在 `tokio::spawn` 里写库，失败只留一行 `tracing::warn!`，
**请求照常返回 200**。表现是"这个操作没有审计记录"，而不是"审计写不进去"。

修法（`migrations/015_widen_audit_action.sql`）：`action` → `VARCHAR(512)`，
中间件再加 `MAX_ACTION_LEN = 512` 截断。
**改列宽而不是只截断**：action 的唯一用途就是检索，一条被截掉尾部的 action 检索不到，等于没有。

#### 🐛🐛 一个夹具泄漏，表现为"随机大面积 403"——本轮最费时间的一处

全量集成测试首跑 **53 条红**，几乎全是 `缺少权限：system:user:create` 之类的 403；
而**单条跑全绿**。逐层查到：

1. 库里 `admin` 用户的角色被改成了 `user`（`user_roles` 里 admin→admin 那行不见了）。
2. 罪魁是 `the_user_list_can_be_filtered_by_role_and_by_active_status`——
   它造 3 个 `filt_user_*` 加 1 个 **`filt_admin_role_*`（持 admin 角色）**，
   测完只 `cleanup_operator`，**这 4 个账号一个都没删**。
   `the_filtered_user_list_count_matches_the_filter` 同样漏了 2 个 `countme_*`。
3. 为什么后果这么重：`ensure_not_last_admin` 判的是 `count_users_with_role("admin") <= 1`，
   那是**全库**计数。多一个 admin 账号 → 守卫认为"还有别人是 admin"而**放行**降级 →
   `last_admin_cannot_be_demoted_or_deleted` 把真 admin 降掉 →
   之后**每一条**用例都因 admin 掉权而红。

守卫本身没坏，**它的成立前提被夹具破坏了**，而这个前提从未写进任何测试。
修法两条：
- 两条筛选测试补齐账号清理（`user_id_by_name` + `delete_user`）。
- `last_admin_cannot_be_demoted_or_deleted` **开头先查全库 admin 列表并断言恰好是 `["admin"]`**，
  失败信息直接点名"是某个夹具没清理"。让前提出现在测试里，而不是靠运气。

教训与 v0.19.0 那次"19 处夹具堆 212 个角色"同源：**夹具泄漏的症状可以离病因一百多条用例**。
新写夹具的判据因此补一条：**只要建出来的账号持有 admin 这类内置角色，就必须清理**——
`opf_*` 前缀守卫只圈 operator 夹具，圈不到这种藏在业务用例里的。

#### 一个仍然存在的、被显式记录的行为

`admin` 用户行与 `admin`/`user` 角色行的 `created_at` 完全相同（07:26:48），
说明它们是**种子**建的；而 `admin` 的 `user_roles` 关联带的是后来那次降级的时间戳。
换句话说种子只建角色与用户行，**角色关联是首次登录或首次建号时补的**。
这不是本轮引入的，但值得记住：迁移重建库后 `user_roles` 可能短暂为空。

### 收尾：M6 门禁全绿 + 本地提交（**未推送**）

门禁结果：`cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` 0 warning /
92 单元 / 12 非集成 / **162 集成 passed**（`--ignored --test-threads=1`）/
`pnpm lint` 0 errors（1 既存 warning：`env.d.ts` 的 `no-explicit-any`）/
`pnpm typecheck`（`vue-tsc`）/ `pnpm test` 161 passed / `pnpm build`。

**注入验证共 5 处**，每处都是"注入必红 → 回滚即绿 → 查库清残留"：
015 迁移的列宽、解锁的 email 桶、profile 的清空 flag、头像的 multipart 拒绝、会话的 404。

**部署侧改动**（本轮新增）：`Dockerfile` 里 `mkdir -p /app/uploads/avatars` + `chown app` +
`VOLUME`；`docker-compose.yml` 加 `uploads:/app/uploads` 命名卷与 `UPLOAD_DIR`；
`.env.example` 与 README 环境变量表补 `UPLOAD_DIR` / `UPLOAD_MAX_FILE_SIZE`；
`.gitignore` 忽略 `uploads/`（开发机上传的头像否则会进版本库）。

**未推送**：按用户既有规矩，本地提交后等明确指令再统一推送。

**接手者注意**：测试库 `axum_api_test` 是长驻共享库，跑集成必须 `--test-threads=1`。
若发现大量 403，先查 `SELECT r.name FROM user_roles ur JOIN roles r ... WHERE ur.user_id = <admin_id>`
——admin 被降级会让**每一条**用例都红，而病因通常在几百行之前的一个漏清理夹具里。

### 里程碑：规划交付完成（未写任何实现代码）

**改了什么**：`docs/ROADMAP.md` 新建（156 行）；`NEXT_VERSION_SCOPE.md` 顶部加封存指引（**历史内容保留未删**）；
本文件头部加本节。

**v0.20.0 结论**：主题定为「账号自持 + 管理可应急」，6 项 = A 线自助（profile 端点 / 列表按角色状态筛选 /
头像上传）+ B 线应急（管理员解锁 / 在线会话列举与单吊销 / CSV 批量导入）。
顺序 M0 发布元数据 → M1 迁移 014 → M2+M3 并行 → M4 会话 → M5 头像上传（放最后，动基础设施）→ M6 前端与门禁。

**两处需要用户拍板的**：
1. v0.19.0 的 tag / CHANGELOG / 版号要不要补。倾向补到 0.20.0 一起抬。
2. 头像存储选型。本计划建议本版走**本地磁盘 + `tower-http` ServeDir**，对象存储抽象推到 v0.22.0。

**下一步**：等指令。未获指令前不写实现代码。按用户既有规矩，本地提交后**不推送**。

### 长驻进程（轮次间会被重置，需要时重新起）

- 后端 `127.0.0.1:8080` 跑 `axum_api_manual` 演示库，env 在 `/tmp/v019/demo.env`
- 前端 vite `127.0.0.1:3000` 代理到 8080；账号 `admin/admin123`
- PG 55432 / Redis 56379。必须带 `RATE_LIMIT_IP_MAX=100000` / `RATE_LIMIT_USER_MAX=100000`，否则整轮 e2e 因限流假红
- 集成测试须 `--test-threads=1`

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


---

# v0.11.0 — 登录可审计 + 自助改密 + 口令策略

## 当前目标

补上"安全追溯的基本盘"。前三版修的都是授权与数据一致性，这一版修的是
**出事之后能不能查**：`/api/auth/login` 与 `/api/auth/register` 在 `public_routes` 里，
**没有挂 `audit_log_middleware`**，因此登录成功、登录失败、注册全部**不进 `audit_logs`**。
失败只进 Redis 计数器，而计数器**带 TTL 会过期**——事后追不出"谁在何时从哪尝试登录"。

同时给用户一条不依赖管理员的改密路径：现在改密只能靠管理员 `reset-password`，
用户被管理员重置才知道自己该改密码。

## 当前计划

| 步骤 | 内容 | 状态 |
|---|---|---|
| 0 | 记录目标、关键决定与起始 git 状态 | ✅ 已完成 |
| 1 | 登录/注册落审计（成功 + 失败 + client_ip） | ✅ 已完成 |
| 2 | 自助改密端点 + 改密后吊销存量会话 | ✅ 已完成 |
| 3 | 口令策略：复杂度下限 | ✅ 已完成 |
| 4 | 首次登录强制改密（受限令牌，不叠加第二次强制登出） | ✅ 已完成 |
| 5 | 前端个人中心 + 强制改密页接线 | ✅ 已完成 |
| 6 | 契约测试 / 集成测试 / e2e / 缺陷注入 | ✅ 已完成 |
| 7 | 质量门禁 | ✅ 全绿 |
| 8 | 文档：CHANGELOG / README / 版本号 | ✅ 已完成（0.10.0 → 0.11.0） |
| 9 | 提交（**不推送**） / tag / Release | 提交 ✅；tag 与 Release 待用户指令 |

### 门禁结论（本轮实跑）

| 项 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅（本轮补跑，发现上一会话遗留的未格式化差异） |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | ✅ |
| `cargo test --locked --lib` | ✅ 61 passed |
| `cargo test --locked --test api_integration -- --ignored --test-threads=1` | ✅ **102 passed**（fresh DB） |
| 前端 typecheck / lint / test / build | ✅ 93 passed；lint 仅剩 `env.d.ts` 既有 warning |
| `node e2e/run.mjs` | ✅ **5/5 套件**（新增 v011，28 条断言） |
| `node e2e/probe-write-guards.mjs` | ✅ **41/41** |

### 缺陷注入（三条全部承重）

| 注入 | 转红的测试 |
|---|---|
| 摘除 `auth_middleware` 的 `pwd_stale` 闸门 | `an_admin_created_user_is_confined_to_changing_password` |
| `AuthService::audit` 变成空操作 | 5 条审计用例 |
| 摘掉 `ChangePasswordRequest` 的 `deny_unknown_fields` | `changing_password_cannot_also_grant_itself_roles` |

### 本轮修掉的三个真实缺陷（都不是测试写错）

1. **改密后前端仍调 `/auth/logout`**，必然 401。
   改密成功时后端已吊销全部会话，那次往返注定失败，
   外加控制台一条 `Failed to load resource`。拆出 `clearLocalSession()`（不发请求）。
2. **受限用户进个人中心刷 4 个「权限不足」**。
   路由守卫明知菜单必然 403 仍去加载菜单与权限码。改为在守卫里直接 return。
3. **`password.ts` 用 `[^\x00-\x7F]` 触发 eslint `no-control-regex`**。
   换成语义完全一致的 `[^\p{ASCII}]`（已用 node 逐样例比对等价）。

### ⚠️ 本轮踩到的坑：集成测试必须带 `--test-threads=1`

漏掉它时并行跑出 2–3 条红（`role_list_pages_over_the_same_set_as_one_big_page` 等），
根因是**共享测试库**下别的用例在两次读之间插/删角色，
断言比较的是两次不一致的快照。加上 `--test-threads=1` 后 102/102 全绿。
README 与本文件早已写明这条规矩，是本轮自己漏了——**不是新缺陷**。

### ⚠️ e2e 侧同样需要"激活"夹具（上一会话只改了集成测试侧）

v0.11.0 让管理员建号带上"强制改密"，于是"建号后直接拿令牌打接口"整条路径失效。
集成测试侧上一会话已加 `activated_token`，**e2e 侧漏了**，
表现为 `role-assignment-guard` 与探针在一堆与被测无关的地方红。
现在 harness 加了 `s.activatedToken(user, 初始口令, 新口令)`：
走**真实产品流程**（自助改密 → 重新登录）拿正常令牌，
不直连数据库改标记，也不依赖测试库实现。

### 顺手量到的基线（非 v0.11.0 引入，仅备查）

e2e 期间后端日志报 `UPDATE users SET password_hash` 耗时 3.5s，
干净环境复测并不复现：

| 操作 | 耗时（**debug 构建**） |
|---|---|
| 登录（1 次 Argon2 校验） | 0.83s |
| 改密（校验 + 哈希 + UPDATE + 吊销） | 1.65s |

3.5s 那次是 e2e 连跑时的锁争用，不是基线缺陷。
口令哈希用的是 `Argon2::default()`（OWASP 推荐档：19 MiB / t=2 / p=1），
**没有改动**——调它属于安全参数取舍，不该顺手改。
上面 0.8s 是未优化的 debug 构建开销，release 下会明显更低。
若日后要优化登录延迟，先量 release 构建的基线，别拿 debug 数字下结论。

### 另一个构造性 flaky：固定 sleep

v011 改密后用固定 `sleep(2500)` 等跳转。单跑够用，全量跑时机器更满就不够——
截图里按钮还在转圈就断言了。已改为 `waitFor(location.pathname === '/login')`。
**固定 sleep 等状态是 e2e 里最常见的假红来源**，新套件一律用 `waitFor`。

## 起始 git 状态

- 分支 `master`，与 `origin/master` 同步
- HEAD = `a0ac25ce feat(v0.10.0): 停止说谎……`
- 工作区：仅本文档改动
- 版本号（起始）：Rust / frontend 均 `0.10.0`
- 版本号（收尾）：Rust / frontend 均 `0.11.0`；`v0.9.0` 已发布，`v0.10.0` 与 `v0.11.0` 均**已提交未推送**

## 关键决定（动手前先定，避免中途反复）

1. **登录审计不能靠挂中间件**。`audit_log_middleware` 注册在认证中间件之内，
   依赖 `AuthenticatedUser` 扩展；而登录请求**本来就没有已认证用户**，
   失败时更没有。所以登录/注册审计必须在 handler / service 里**显式写入**。
   反过来说，中间件那条路径对登录是**结构性不适用**，不是"忘了挂"。

2. **登录审计的 `action` 用语义值，不用 `"{METHOD} {path}"`**。
   登录失败要和登录成功能被区分（否则事后无法回答"有没有人在爆破"），
   因此约定 `AUTH_LOGIN_SUCCESS` / `AUTH_LOGIN_FAILURE` / `AUTH_REGISTER`，
   并在 `result` 列写明失败原因（账号不存在 / 口令不符 / 账号停用 / 已锁定）。
   注意这**偏离**了中间件的 `{METHOD} {path}` 格式，是有意的——
   登录的"方法+路径"三行都一样，只有结果与身份有区分度。

3. **登录审计必须同步 `await` 写入，不能 `tokio::spawn`**。
   中间件那样做是为了不拖慢响应；登录是低频且**安全关键**路径，
   写失败必须让 `login` 报错而不是静默丢失——否则"审计"又变成一个
   "失败时无声"的能力。代价是登录多一次 INSERT 往返，接受。

4. **口令策略只在"设置口令时"生效，登录时不校验**。
   否则把复杂度下限一抬，**存量弱口令用户当场被锁在门外**。
   长度下限 6 → 8，且要求至少 2 类字符（大写/小写/数字/符号）。
   选的门槛必须让 `admin123` 通过——它是 README 与 e2e 的默认账号，
   卡住它等于卡住整个测试套件和首次部署。

5. **首次强制改密用"受限令牌"，不叠加第二次强制登出**。
   handoff 明确警告过：`iat_ms` 升级已经让存量令牌作废过一次。
   再来一次"登录即踢下线"是第二次同类冲击。做法：
   `users.must_change_password` 落库；登录时若为真，令牌带 `pwd_stale` claim，
   `auth_middleware` **只放行改密/登出/`/me`**，其余一律 403 并提示改密。
   用户改完密拿到正常令牌，全程不丢工作、也不需要重新输密码。

6. **`must_change_password` 的默认值必须是 `FALSE`**。
   迁移给存量用户补列时默认 false，**存量用户完全不受影响**——
   这是"不叠加第二次强制登出"的关键。只有管理员**新建/重置**的用户才置 true。

7. **自助改密必须验旧口令，且新口令不能与旧口令相同**。
   只验新口令复杂度是不够的：拿到一个劫持来的令牌就能把密码永久改掉。
   "新口令与旧口令相同"也要拒绝，否则改密是空操作却给了"已改密"的假象。

8. **前端路由由后端菜单驱动，个人中心不能走菜单**。
   `buildRoutesFromMenus` 只注册 `menus.permission` 里有的页面；
   个人中心是**所有登录用户**都该有的页面，不该塞进按角色授权的菜单表
   （那会让"角色没勾这个菜单"的用户直接没有个人中心）。
   做法：在 `router/index.ts` 的 `MainLayout` children 里**静态注册**，
   与 `/login` `/register` 同样的白名单式处理。

---

# v0.12.0 —— 错误响应格式统一（入参不合法，任何端点都得长一个样）

## 当前目标

同一个"入参不合法"，走 `Query` 得到 400 + JSON，走 `Json` 得到 422 + `text/plain`，
走 `Path` 得到 400 + `text/plain`。**同一类错误因端点不同而形状不同**，
而前端响应拦截器按 `message` 取文案（`frontend/src/api/index.ts`），
纯文本那一种取不到 `message`，用户只能看到一个空错误框。

这是 v0.10.0「界面不许说谎」的后端侧同族问题：**对外承诺了统一格式，
但框架默认路径绕过了它**。v0.11.0 记下这个遗留，本版收掉。

## 当前计划

| 步骤 | 内容 | 状态 |
|---|---|---|
| 0 | 记录目标、关键决定与起始 git 状态 | ✅ 已完成 |
| 1 | `ApiPath<T>` 提取器（与 `ApiJson<T>` 同构） | ✅ 已完成 |
| 2 | 19 处 `Json<T>` → `ApiJson<T>` | ✅ 已完成 |
| 3 | 17 处 `Path<T>` → `ApiPath<T>` | ✅ 已完成 |
| 4 | 承重测试：遍历 OpenAPI 全路由，坏输入必须回统一信封 | ✅ 已完成（54 条探针） |
| 5 | 缺陷注入 | ✅ 已完成（两处注入均被抓） |
| 6 | 文档：CHANGELOG / README / 版本号 | ✅ 已完成 |
| 7 | 全量质量门禁 | ✅ 全绿 |
| 8 | 提交（**不推送**） | ✅ 已完成 |

## 起始 git 状态

- 分支 `master`；HEAD = `7d6898ff feat(v0.11.0): 登录可审计 + 自助改密……`
- 工作区干净（仅本文档改动）
- 待推送：v0.10.0 (`a0ac25ce`) + v0.11.0 (`7d6898ff`)，**均未推送**
- 版本号：Rust / frontend 均 `0.11.0`

## 缺口清单（本轮**实测**得到，不是照文档推测）

起真实后端逐个打了一遍：

| 入口 | 现状 | 坏输入的实际响应 |
|---|---|---|
| 19 处 `Json<T>` | 绕过 `AppError` | `400`/`422` + `text/plain` |
| 17 处 `Path<T>`（16 × `Path<Uuid>` + 1 × `Path<String>`） | 绕过 `AppError` | `400` + `text/plain` |
| Content-Type 不是 JSON | 绕过 `AppError` | `415` + `text/plain` |
| 5 处 `Query`（已有 `From<QueryRejection>`） | ✅ 已统一 | `400` + `application/json` |
| 1 处 `ApiJson`（v0.11.0 改密端点） | ✅ 已统一 | `400` + `application/json` |

原始响应示例（v0.11.0 前后对照）：
```
POST /api/admin/roles  body='{bad json'
  → 400 text/plain  "Failed to parse the request body as JSON: key must be a string at line 1 column 2"
PUT /api/auth/password  body='{bad json'
  → 400 application/json  {"code":400,"message":"错误的请求: 请求体不合法: Failed to parse..."}
```

## 关键决定（动手前先定）

1. **承重测试必须能抓出"漏改的第 37 处"**。
   36 处机械迁移最容易出的错就是漏一处，而漏一处不会有任何编译错误。
   因此测试不写成"断言这 36 处都改了"（那是自证），
   而是**遍历 OpenAPI 全部路由**，对每个写端点发一个坏请求体、
   对每个含 `{id}` 的路径发一个非 UUID 参数，断言响应一定是
   `application/json` 且含 `code`/`message`。这样以后新增端点忘了用
   `ApiJson`，测试当场变红。判据落在**可观测的响应形状**上，不是源码文本。

2. **415 并入 400，不保留 415**。
   HTTP 语义上 415 更准确，但 v0.11.0 的改密端点**已经发布**并返回 400。
   同一个逻辑错误因端点不同而返回不同状态码，正是本版要消灭的问题；
   要改就得连同已发布行为一起改，那是破坏性变更，不该顺手做。
   消息文本里已说明"必须带 Content-Type: application/json"，足够调用方分辨。

3. **`ApiPath<T>` 与 `ApiJson<T>` 同构，不合并成一个泛型包装器**。
   `JsonRejection` 有 4 个变体需要分别翻译（尤其 415 要给专门文案），
   `PathRejection` 只是一条消息。强行合并成一个 `Api<T>` 泛型，
   内部仍然要 `match` 分派，只是把分支藏得更深，收益不抵可读性损失。

4. **只迁移，不改业务语义**。
   状态码从 422 变 400 是本版唯一的语义变化，且是**修正**：
   422 在本项目里只可能来自 `Json` 的反序列化失败（`Query` 已是 400），
   同一个"入参不合法"给两个码没有信息量。

5. **不碰 `GET` 的响应体**。
   导出的 Excel/CSV、二进制响应天然不是 JSON，那不是"错误格式不统一"。
   承重测试只针对**入参**错误，不针对成功响应。



## v0.12.0 实施记录（步骤 1–6 已完成，门禁进行中）

### 实际改动

| 文件 | 改动 |
|---|---|
| `src/utils/json_extractor.rs` → `src/utils/api_extractor.rs` | `git mv` 更名；新增 `ApiPath<T>` + `map_path_rejection` |
| `src/controller/{auth,demo,dict,menu,role,user}.rs` | 19 处 `Json<T>` → `ApiJson<T>`，17 处 `Path<T>` → `ApiPath<T>` |
| `src/controller/role.rs` | 顺带修 `RoleListParams` 的 `parameter_in`（见下） |
| `src/middleware/permission.rs`、`src/controller/role.rs` | 两处提到「422」的注释更正为 400 |
| `tests/api_integration.rs` | 新增 `every_bad_input_returns_unified_error_envelope` + 6 个辅助函数/结构 |
| `Cargo.toml` / `Cargo.lock` / `frontend/package.json` | 0.11.0 → 0.12.0 |
| `CHANGELOG.md` / `README.md` | 新增 v0.12.0 条目与能力清单行 |

### 踩到的坑（都已在源码注释里留痕）

1. **`Path<T>` 实现的是 `FromRequestParts` 不是 `FromRequest`**。
   所以 `ApiPath` 的签名是 `from_request_parts(parts: &mut Parts, state: &S)`，
   `use` 里要的是 `http::request::Parts`，不是 `http::Request`。

2. **`#[utoipa::into_params(...)]` 不存在**。`into_params` 是 `IntoParams` derive
   的 **helper attribute**，要写裸的 `#[into_params(parameter_in = Query)]`。
   写成 `#[utoipa::into_params(...)]` 报 `could not find into_params in utoipa`。

3. **`PathRejection` 与 `JsonRejection` 都是 `#[non_exhaustive]`**，
   `match` 必须留 `other =>` 兜底分支，否则 axum 小版本加变体就直接编译不过。
   本项目的选择是显式承认"未知形态"并回统一格式，而不是让升级失败。

4. **`ops.sort()` 在 `(String, String, Value)` 元组上报 `JsonValue: Ord` 不满足**。
   `sort()` 要求整个元组可比较，必须改用 `sort_by(|a, b| a.0.cmp(&b.0).then(...))`。

5. **`Request<Body>` 不是 `Clone`**，探针表不能复用同一个请求两遍；
   现在探针按值消费。

### 承重测试设计（为什么它抓得到"漏改的那一处"）

- 探针表由 `docs::openapi_json()` **驱动**：遍历 `paths` → 每个操作，
  读 `requestBody` 是否存在、`parameters` 里 `in: path` 且 `schema.format == uuid` 的参数名。
  **不手写端点清单**——手写清单只能证明清单里那几处迁移了。
- 三类探针 → 19（坏 JSON）+ 19（错 Content-Type）+ 16（非 UUID 路径）= **54 条**
- **必须带合法 admin 令牌**：鉴权中间件先于提取器跑，无令牌拿到 401，
  而 401 也是 JSON 信封 → 断言会"因为错误的原因而通过"
- 三条**探针表自检**（`>= 19` / `>= 19` / `>= 15`）：防止 OpenAPI 结构变化导致
  一条都没解析出来、循环空转、断言全绿
- 探针三只在**带 uuid 参数**时打：`{code}` 是 `String`，塞什么都能解析出来，
  拿它探只会得到 200/404，测不到提取器

### 缺陷注入（两条都承重）

| 注入 | 结果 |
|---|---|
| `user.rs` 的 `batch_delete` 退回裸 `Json<BatchDeleteRequest>` | 用例红：报出该端点 `400 text/plain` + `415 text/plain` |
| `dict.rs` 的 `update_item` 退回裸 `Path<Uuid>` | 用例红：报出该端点 `400 text/plain` |
| 恢复两处 | 用例绿 |

失败信息会指出是哪个端点、哪种探针、三条不合格原因分别是什么
（状态码 / Content-Type / 响应体结构），不用回去翻代码定位。

### 顺带发现的真实文档缺陷（已修）

`GET /api/admin/roles` 的 `page` / `page_size` 在 OpenAPI 里是
`in: "path"`、`required: true`，但路径模板 `/api/admin/roles` 里没有 `{page}`。

根因链：utoipa 的 `axum_extras` 本该从 handler 参数推断 `parameter_in`，
但 `list_roles` 的签名是 `Result<Query<RoleListParams>, QueryRejection>`
（显式接住拒绝，为了走 `From<QueryRejection>`），utoipa 认不出裸 `Query<...>`，
于是回落到 `ParameterIn::default()`——**而那个默认值是 `Path`**
（`utoipa-5.5.0/src/openapi/path.rs` 的 `impl Default for ParameterIn`）。

修法：`#[into_params(parameter_in = Query)]`。全项目只有这一处 `IntoParams` derive，
所以影响面就这一个端点。**教训**：utoipa 的默认值不能当"正确的默认值"用，
凡靠推断的地方都要在生成结果里核对一眼。


## v0.12.0 门禁结论（全绿）

| 项 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | ✅ 零警告 |
| `cargo test --locked --lib` | ✅ 61 |
| 集成测试（fresh DB + `--test-threads=1`） | ✅ **103**（v0.11.0 为 102，+1 即新增承重测试） |
| 前端 lint / typecheck / Vitest / build | ✅ / ✅ / ✅ 93 / ✅（lint 1 个既有 warning 在 `env.d.ts`，与本版无关） |
| e2e 真实 Chrome | ✅ 5/5 套件 |
| 授权探针 | ✅ 41/41 |

**真实服务上复核**（起 8080 连测试库，逐条 curl 确认非仅测试环境）：

```
POST /api/admin/roles  body='{bad json'
  → 400 application/json {"code":400,"message":"错误的请求: 请求体不合法: Failed to parse the request body as JSON: key must be a string at line 1 column 2"}
POST /api/admin/roles  Content-Type: text/plain
  → 400 application/json {"code":400,"message":"错误的请求: 请求体不合法: 请求必须带 Content-Type: application/json"}
GET  /api/admin/users/not-a-uuid/roles
  → 400 application/json {"code":400,"message":"错误的请求: 路径参数不合法: Invalid URL: Cannot parse `user_id` with value `not-a-uuid`: UUID parsing failed..."}
```

`/api/openapi.json` 的 `info.version` 已随 Cargo 派生为 `0.12.0`（无需手写同步）。

### 提交

`feat(v0.12.0): 错误响应格式统一——入参不合法，任何端点长得一样`，
17 个文件、+730/-134。**未推送**。

## 待推送队列（累计 3 个提交）

| 提交 | 版本 |
|---|---|
| `a0ac25ce` | v0.10.0 |
| `7d6898ff` | v0.11.0 |
| 本次 | v0.12.0 |

按指令：本地跑门禁 + 提交，**全部工作完成或收到指令才统一推送**。


---

# v0.13.0：审计要能回答"改了什么"

## 当前目标

v0.11.0 让登录落审计，解决的是"谁登录过"。但审计系统整体仍只回答
**「谁在什么时候调了哪个接口」**，回答不了**「改了什么」**。

对管理后台来说，后者才是事后追溯的真正问题：出事后要回答的是
"这个角色叫什么、被谁删的"、"谁给谁授了哪些权限码"、"这个账号的
状态被谁改的"——而这些答案现在在库里**不存在**。

## 实测证据（本轮打出来的，非推测）

审计表的 `params` / `result` 两列，对**所有写操作恒为空**：

```
 action                                                     | params | result
 DELETE /api/admin/roles/a2b5e722-a23f-4a8c-a55d-27752723c090  |        |
 PUT  /api/admin/roles/7d749748-.../menus                       |        |
 POST /api/admin/users                                         |        |
```

`params` 只在有查询串时才有值（GET 筛选）；写操作的请求体**按设计不记录**
（口令、令牌入库即长期泄露面，这个取舍本身是对的，见
`middleware/audit_log.rs` 开头）。于是结果就是：

1. **角色被删后名字永久丢失**。审计只剩一个 UUID，而 `roles` 行已删除，
   无处可查这个角色叫什么、有什么权限。事后连"删的是什么"都答不出。
2. **授权授予无法复盘**。`PUT /roles/{id}/menus` 只知道"某角色的菜单被改了"，
   不知道授了/撤了哪些权限码。而这是整个系统里风险最高的操作。
3. 全库 8 张表（`users`/`roles`/`role_menus`/`menus`/`dict_*`/`audit_logs`），
   **没有任何变更历史表**，上述信息不存在别处。

## 顺带实测到的三处"文档/注释说谎"（同族，低成本，一起修）

| 位置 | 声称 | 实际 |
|---|---|---|
| README 能力清单 ⚠️ 行 | 「非 admin 角色仍被 `require_role("admin")` 整体挡住」 | `require_role` 连同 router 里 5 处调用**已在 v0.5.0 PR-3 整体删除**（`middleware/auth.rs:6` 有记录）。这条把一个**不存在的闸门**说成现存边界，方向还是反的 |
| README 限流行 | 「固定窗口限流（IP 维度）」 | 实际 IP + **用户**双维度（`rate_limit.rs:78` 用 `user_max_requests`），README 少说了一半 |
| `frontend/src/utils/storage.ts:11` | 「简单的 **XOR** + Base64 编码」 | 实现里**没有 XOR**，只有 `btoa(encodeURIComponent(...))`，纯 Base64 |

## 关键决定（动手前先定）

1. **不记录请求体，改用 handler 显式声明摘要**。
   自动记录请求体会把 `password` / `old_password` / `new_password` 写进长期表，
   那是把"少记"换成"泄密"。改为：写操作的 handler 主动往审计摘要里追加
   **它自己知道安全的那部分**（资源名、授予的权限码列表、状态变化）。
   不写就不入库——默认安全，而不是默认危险。

2. **摘要走请求扩展，不改 handler 返回类型**。
   新增 `AuditDetail`（`Arc<Mutex<Vec<String>>>` 挂在 extensions 上），
   中间件在写库前合并进 `result` 列。handler 签名不因此变化，
   提取器可省略——不写摘要的端点行为不变，只是摘要为空。

3. **"删的是什么"由 helper 在删之前查一次名字**。
   资源被删掉之后，名字只能靠删除前留痕。与其让每个 handler 各写一遍，
   不如提供 `audit_label(state, "角色", id)` 这类助手，统一格式
   （如 `角色 "admin"`）。

4. **摘要必须能被集成测试断言**，否则又是一个"以为记了"的字段。
   承重测试逐个走授权写入口，断言审计里能查到资源名/权限码。

5. **不改审计的中层设计**：不引入独立变更历史表。
   一张 `audit_logs` 够用——授权变更的语义摘要写进 `result`，
   比新增一张 `permission_changes` 表再写双份更容易保持一致。

## 当前计划

| 步骤 | 内容 | 状态 |
|---|---|---|
| 0 | 记录目标、决定与实测证据 | ✅ 已完成 |
| 1 | `AuditDetail` 扩展 + 中间件合并进 `result` | 待做 |
| 2 | `audit_label` helper（删除前留资源名） | 待做 |
| 3 | 授权写入口接入摘要（角色 CRUD / 菜单授权 / 用户角色绑定） | 待做 |
| 4 | 用户写入口接入摘要（增删改 / 状态切换 / 重置口令） | 待做 |
| 5 | 承重测试：授权变更后审计必须能答出"谁授了什么给谁" | 待做 |
| 6 | 缺陷注入 + 质量门禁 | 待做 |
| 7 | 修三处"文档/注释说谎" + CHANGELOG / README / 版本号 | 待做 |
| 8 | 提交（**不推送**） | 待做 |

## 起始 git 状态

- 分支 `master`；HEAD = `b64135d4 feat(v0.12.0): 错误响应格式统一……`
- 工作区干净
- 待推送 3 个：v0.10.0 `a0ac25ce` / v0.11.0 `7d6898ff` / v0.12.0 `b64135d4`
- 版本号：Rust / frontend 均 `0.12.0`

---

# v0.13.0 进展（会话续接记录）

## 已完成：步骤 1–2

**步骤 1 — `AuditDetail` 扩展 + 中间件合并**（`src/middleware/audit_log.rs`）

- 新增 `AuditDetail`：`Arc<Mutex<Vec<String>>>`，`FromRequestParts` 提取器
- 中间件在 `next.run` **之前**把它挂进 extensions，handler 取到的是同一个 `Arc`
- 写库前 `render()` 合并多条摘要 → `audit_logs.result` 列
- **只在 2xx 时合并摘要**：handler 追加后仍可能失败（5xx），此时写进去就是谎报
  "已授予/已删除"。审计一旦开始说谎比没有审计更危险——它会让人**不再去看**其他证据
- `MAX_RESULT_LEN = 2000`：`result` 是无限制 `TEXT`，权限码一多能写几十 KB
- 查名字失败**不冒泡成 500**，退化成 `角色 <uuid>`（旁路能力不该拖垮业务请求）

**步骤 2 — `utils::audit` 助手**（`src/utils/audit.rs`）

- `label(kind, name)` → `角色 "admin"`（ASCII 引号，便于复制进工单/SQL）
- `role_label` / `user_label` / `menu_label` / `dict_type_label` / `dict_item_label`：
  **删之前查一次名字**，名字只存在行里、删掉就没了
- `diff_summary(before, after, granted_prefix, revoked_prefix)`：授权变更只记**差异**
  （重复提交同一份授权不是变更，记成变更会给"谁动过这里"制造假阳性）
- `codes()`：空集合返回空串 → 调用方整条省略；超 12 条用"N 个"概括且**说清总数**
- 为此把 `dict_repo::find_type_by_id` / 新增 `find_item_by_id` 开放为 `pub`

编译通过。当前 git 状态：工作区有 v0.13.0 步骤 1–2 的改动（未提交）。

## 下一步

步骤 3：授权写入口接入摘要（角色 CRUD / 菜单授权 / 用户角色绑定）。

## 已完成：步骤 3–7

**步骤 3–4 授权与用户写入口接入摘要**（25 个写端点全覆盖）

风险最高的两条单独说明：

- `assign_role_menus`（`menu.rs`）：新增 `MenuRepository::permission_codes_of_role`，
  替换前后各取一次快照求差 → 审计记"授予权限码 A、B；撤销权限码 C"。
  只记提交上来的集合答不出"撤了哪些"，而撤销恰是事后追溯最想知道的一半
- `delete_role`：删前已用 `FOR UPDATE` 取出 `name`，直接落摘要；
  同时记下该角色承载的权限码（删角色 = 把这些码从所有人身上撤走）
- `update_user`：记角色增删差异 **与** 状态前后变化
- `batch_delete_users`：逐个记名字，而不是只记"删了 1 个"

**顺带发现并修掉一处新的"界面说谎"**：`audit_logs.result` 既不在界面表格里、
也不在 Excel 导出里——摘要存进库却读不到，等于没做。
前端日志页与导出列都补了"变更摘要"。

**步骤 5 承重测试**

- `every_write_operation_leaves_an_answerable_change_summary`：逐个走完 25 个写端点，
  断言审计里能读到资源名/权限码差异/口令重置对象，并断言**口令一个字都不入库**
  （扫 `params` 与 `result` 两列，只查 `result` 不够——`params` 是查询串，
  有人改成记请求体时秘密就从那里漏）
- `every_documented_write_operation_is_covered_by_the_audit_test`：从 OpenAPI 派生
  写操作集合做清单自检。**第一次运行就抓到了自己漏掉的 `DELETE /api/admin/users/{id}`**
- `a_rejected_write_leaves_no_change_summary`：被拒的写操作不留摘要
- 单测 7 条（`utils::audit`）+ 6 条（`middleware::audit_log`）

**步骤 6 缺陷注入（两条）**

| 注入 | 结果 |
|---|---|
| `delete_role` 不再记摘要 | ✅ 承重用例变红 |
| 去掉 2xx 门禁 | ❌ **没被抓到** |

第二条没抓到是重要发现：现有 handler 全都把 `push` 放在所有副作用成功之后，
被拒请求根本走不到 push，所以 2xx 门禁在接口层**不可达**。
处理方式不是删掉门禁，而是把它抽成纯函数 `summary_for` 并用单测钉住两条分支，
同时把集成测试的注释改成诚实版本（它承重的是 push 时机，不是门禁）。
重新注入后单测变红。**不让注释里的说法比实际验证到的更强。**

**顺带查明（非缺陷）**：`code_a → code_b` 的改码路径在接口层走不通——
`update_menu` 要求持有目标码，目标码已存在时又撞唯一索引 409，两道守卫互相堵死。
该摘要分支无法被集成测试触达，改为 `audit::permission_change` 单测承重。

**步骤 7 文档**

- CHANGELOG 0.13.0；版本号 Cargo / frontend 均 → `0.13.0`（OpenAPI 由 Cargo 派生）
- 修三处说谎：README ⚠️ 行（`require_role` 已删）、限流行（实为 IP+用户双维度，
  补 `RATE_LIMIT_USER_MAX`/`RATE_LIMIT_USER_WINDOW`）、`storage.ts`（无 XOR）

## 已完成：步骤 8 门禁（全部跑完，含此前从未统计过的那一组）

| 门禁 | 结果 |
|---|---|
| `cargo fmt --check` | ✅ |
| `cargo clippy --all-targets -- -D warnings` | ✅ 零警告 |
| 单测 | ✅ **74** |
| 集成 `--ignored --test-threads=1` | ✅ **105** |
| 集成 非 ignored 组 | ✅ **8**（此前从未单独跑过，见下） |
| 前端 lint | ✅ 0 error（1 个**既有** warning：`frontend/env.d.ts` 的 `any`，非本次改动） |
| 前端 typecheck | ✅ |
| 前端 Vitest | ✅ **93** |
| 前端 build | ✅ |
| e2e | ✅ **6/6** 套件 |
| 授权探针 | ✅ **41/41** |

### ⚠️ 上一版说"集成 103 全绿"，那个说法当时并不成立

`every_query_dto_rejects_unknown_fields`（非 ignored 组之一）**在 v0.12.0 就已经是红的**。
已用 `git worktree` 在 HEAD（v0.12.0）上复核确认。
当时只统计了 `--ignored` 那 105 条，非 ignored 的 8 条从未计入——
包括这条本身。**"全绿"只覆盖了被数过的那部分。**

根因不是代码：`#[serde(deny_unknown_fields)]` 在 `role.rs` 第 39 行，
中间隔着 7 行解释 utoipa 的注释和 `#[into_params(parameter_in = Query)]`，
而测试只取**紧邻上一行**。已改为向上遍历连续的 attribute/注释块，
并把失败信息里带上整个属性块——让下一次失败当场可读。

**教训：门禁的"全绿"必须连同"被数了几条"一起说。**

### ⚠️ e2e 套件 `v010-ui-truth.mjs` 的第二页断言依赖污染过的库

跑 e2e 时它红了：`第二页可点 :: pagerText=1`。
角色列表每页 10 条，而**干净库只有 `admin`/`user` 两个角色**——第二页压根不存在。
该套件过去能过，只因为跑它之前刚跑过集成测试、库被污染出了足够多的角色。
那是**偶然**，不是前提：换个干净库就红，而红的原因与被测的界面毫无关系。

已改为套件自己按当前总数补足到 `page_size + 1`（本次补 9 个），
跑完逐个删除并核对残留 0。核对刻意放在删除**之后**——
先查再删的话，删除动作本身从没被检验过，一个"建了不删"的实现同样能全绿。

### 新增 e2e 套件 `v013-audit-change-summary.mjs`（25 条断言）

v0.13.0 唯一用户可见的产出是"变更摘要"列，此前**没有任何一层门禁在验证它真的渲染出内容**。
接口测试能证明库里那列有内容，却证明不了界面和导出的 xlsx 读得到——
一列加了但取错字段（比如取 `params` 而不是 `result`）时，上面所有测试都照样绿。
套件验：表头在、写操作的摘要格不是破折号、xlsx 的 `sharedStrings` 里既有表头也有摘要内容。

写这个套件时踩到的两个坑，都已写进 `e2e/README.md`：

1. 建用户必须给 `roles`，否则 400「至少需要指定一个角色」
2. 前端把导出名写死成「操作日志.xlsx」，Chrome 遇同名文件是**覆盖**而非加「(1)」后缀，
   所以"目录里多出一个新文件"这种判据永远不成立（上次的残留把判据永久钉死）。改看修改时间。

### e2e README 补了"前置数据必须由套件自己造"这条纪律

## 起始 git 状态（v0.13.0 提交前）

- 分支 `master`，HEAD = `b64135d4 feat(v0.12.0)`
- 待推送 4 个提交（用户指令：**全部完成或收到指令才统一推送**）
  `a0ac25ce` v0.10.0 / `7d6898ff` v0.11.0 / `b64135d4` v0.12.0 / v0.13.0
- 版本号 Rust / frontend 均 `0.13.0`

## 下一步

步骤 8：提交 v0.13.0，**不推送**。提交信息：
`feat(v0.13.0): 审计要能回答"改了什么"`

**仍未推送。** 等用户指令。

---

## v0.14.0 计划：审计会过期，但没人被告知

### 起始 git 状态

- HEAD = `a4486f55 feat(v0.13.0)`，工作区干净（仅 README 门禁说明一处待提交）
- 待推送 5 个提交：`a0ac25ce` v0.10.0 / `7d6898ff` v0.11.0 /
  `b64135d4` v0.12.0 / `a4486f55` v0.13.0 / README 门禁说明

### 缺口（本轮**实测**得到，不是照文档推测）

主题候选来自一个追问：v0.13.0 把"改了什么"唯一地存进 `audit_logs.result`，
而 CHANGELOG 自己写着"全库 8 张表没有任何变更历史表，上述信息不存在别处"。
那么——**这张表自己会被删吗？**会。

实测（起第二个后端实例，`AUDIT_LOG_RETENTION_DAYS=91`、
`AUDIT_LOG_CLEANUP_INTERVAL_SECONDS=2`）：

```
INSERT 一行 created_at = now() - 100 days → 存在
等 5 秒后再查                                → 0 行，被后台任务删掉
```

`AUDIT_LOG_RETENTION_DAYS` **默认 90**，清理每 3600 秒跑一轮，
`DELETE FROM audit_logs WHERE created_at < cutoff` 无条件执行。

| # | 缺口 | 实测依据 |
|---|---|---|
| 1 | 清理动作只进 `tracing::info!` | 进的是进程 stdout；不翻服务器日志就看不到。而"日志没了"恰恰是出事时最需要回答的问题 |
| 2 | 界面查不到保留策略 | `grep -rn retention src/controller/ frontend/src/` → 零命中 |
| 3 | 接口查不到保留策略 | 无任何端点暴露 `retention_days` |
| 4 | README 完全没写这四个变量 | 环境变量表里有 `RATE_LIMIT_*` / `LOGIN_*`，独缺 `AUDIT_LOG_*`。`grep AUDIT_LOG README.md` → 零命中 |
| 5 | 后果没有被告知过 | v0.13.0 之前 `result` 恒为空，删了无所谓；v0.13.0 之后它是唯一副本，于是 91 天后授权变更复盘能力归零，而没有任何一处告诉使用者这件事 |

**用户会遇到的真实场景**：调查一起 3 个月前的授权变更，日志页按日期筛，
什么都没查到——管理员无从判断"是没发生过"还是"发生过但被清了"。
这个歧义本身就是审计的失效。

### 修复方向

| 步骤 | 内容 |
|---|---|
| 1 | 迁移 `011_audit_log_purges.sql`：清理动作自证（截止时间、删除行数、执行时刻、耗时、是否因上限提前收手） |
| 2 | `GET /api/admin/audit-logs/retention` → 保留天数、是否启用、最老一条日志、最近一次清理 |
| 3 | 日志页顶部如实说明保留策略与最近一次清理 |
| 4 | 时间范围筛选早于最老日志时提示，而不是静默返回空 |
| 5 | README 环境变量表补齐 `AUDIT_LOG_*` 四项 |
| 6 | 承重测试 + 门禁 + 提交（**不推送**） |

### 关键决定（动手前先定）

1. **清理记录不能写进 `audit_logs` 自己**。审计表记自己的被删，
   会陷入"删这行要不要连带记录、记录的那行算不算过期"的递归。
   独立小表 `audit_log_purges` 是唯一干净解。
2. **只在真的删了行时才记**。每轮都记会把表变成噪声，
   而"有清理发生"才是需要被看见的事实。
3. **界面提示要给出可行动信息**（保留天数 + 最老日志时刻），
   而不是一句"部分日志可能已过期"——后者等于没说。
4. **不延长默认保留天数**。90 天是合理的默认值，改它只是掩盖问题，
   问题在于"没人被告知"。

### 已完成：步骤 1–6，v0.14.0 门禁全绿

| 门禁 | 结果 |
|---|---|
| `cargo fmt --check` | ✅ |
| `cargo clippy --all-targets -- -D warnings` | ✅ 零警告 |
| 单测 | ✅ **74** |
| 集成 非 ignored 组（空库） | ✅ **8** |
| 集成 `--ignored`（空库） | ✅ **109**（原 105 + 新增 4） |
| 前端 lint | ✅ 0 error（1 个**既有** warning：`frontend/env.d.ts` 的 `any`） |
| 前端 typecheck | ✅ |
| 前端 Vitest | ✅ **105**（原 93 + 新 12） |
| 前端 build | ✅ |
| e2e | ✅ **7/7** 套件 |
| 授权探针 | ✅ **41/41** |

### 缺陷注入三条，全部被承重用例抓到

| 注入 | 结果 |
|---|---|
| `record_purge` 置空（清理不留痕） | ✅ 两条集成用例变红 |
| `hit_batch_limit` 恒报 false | ✅ 撞上限那条变红 |
| 前端文案隐去"撞上限"提示 | ✅ 对应单测变红 |

### 顺带修掉：五条用例"靠别的用例先跑过"

`audit_log_retention_removes_only_expired_rows`、
`audit_log_retention_deletes_in_bounded_batches`、
`one_endpoint_stays_one_row_when_it_is_partly_flushed_and_partly_pending`
以及本次新增的两条 purge 用例，都直接 `INSERT audit_logs` 却不建 app——
迁移由 `create_router` 触发，所以它们此前只在**全量跑**时成立，
靠字母序靠前的用例把库迁移过。单跑任何一条都会撞
`relation "audit_logs" does not exist`。

已加 `ensure_schema()` 守卫，并**逐条在空库上单跑验证**（7/7 通过）。
这与 v0.12.0 那次"非 ignored 组从未计入"是同一族问题：
全量绿、单独红，而后者更难发现。

### 版本号与文档

- 版本号 Cargo / frontend 均 → `0.14.0`
- README 环境变量表补齐 `AUDIT_LOG_*` 四项（此前一个都没有），
  并说明保留策略可经接口查询
- e2e README 登记新套件
- CHANGELOG 0.14.0

### 起始 git 状态（v0.14.0 提交前）

- 待推送 5 个提交（用户指令：**全部完成或收到指令才统一推送**）：
  `a0ac25ce` v0.10.0 / `7d6898ff` v0.11.0 / `b64135d4` v0.12.0 /
  `a4486f55` v0.13.0 / v0.14.0

### 下一步

v0.14.0 提交（**不推送**）。后续版本的缺口需重新实测得出——
v0.14.0 的做法是：从上一版 CHANGELOG 自己写下的"信息不存在别处"这句反问，
再动手实测，而不是照着设想的功能清单往下排。

---

## v0.15.0 计划：会话失效时，界面说的是"404 页面未找到"

### 起始 git 状态

- HEAD = `335daa48 feat(v0.14.0)`，工作区干净
- 待推送 6 个提交：`a0ac25ce` / `7d6898ff` / `b64135d4` / `a4486f55` /
  `335daa48` / v0.15.0。**统一推送的约束仍然有效。**

### 缺口（实测）

方法同 v0.14.0：先从代码与文档的既有断言反问，再用浏览器实测。

反问一：README 环境变量表是否列全了代码里读的所有变量？
`grep -rhoE 'env::var\("[A-Z_]+"' src/` 与 README 对比，
除 v0.14.0 补齐的 `AUDIT_LOG_*` 外还剩 `METRICS_FLUSH_INTERVAL_SECONDS` /
`METRICS_KEY_TTL_SECONDS` / `METRICS_MAX_BUFFERED_ENDPOINTS` 三项未写。
危害较小（指标键 7 天 TTL，且指标不承载追溯责任），**留作顺手修，不单独立项**。

反问二（这条是真缺陷）：前端路由守卫注释写着"加载失败已由 store 弹出提示；
这里放行，由 404 页面兜底，避免守卫死循环"。那**令牌被服务端吊销**时，
菜单接口返回 401，守卫就走进了这条分支。实测：

```
已登录 admin                    → path=/
logout（吊销该会话）             → 200
同一令牌再调 /api/auth/me        → 401「令牌已被注销，请重新登录」
goto /system/user               → path=/system/user   ← 没被带去登录页
页面正文 = "404 页面未找到 返回首页 未授权，请重新登录 未授权，请重新登录
            未授权，请重新登录 未授权，请重新登录"
localStorage 里令牌仍在          → true
```

四个问题叠在一起：

| # | 现象 | 为什么是缺陷 |
|---|---|---|
| 1 | 页面显示 **"404 页面未找到"** | 会话没了 ≠ 页面不存在。这是**方向性相反**的诊断：按 404 去排查会去找根本不存在的路由 |
| 2 | "请重新登录"说了，却不跳登录页 | 用户得自己猜出要去点哪里 |
| 3 | 同一条提示重复 4 次 | 一个请求风暴被当成 4 个独立错误，无从判断 |
| 4 | 令牌留在 localStorage | 之后每次进任何页面都重演一遍，且守卫永远认为"已登录" |

**这条路径在正常产品操作里就会走到**，不是边缘场景：
- v0.9.0 起，改他人角色会吊销其全部会话
- v0.11.0 起，改密会吊销该账号全部会话
- 停用账号同样使其会话失效

也就是说：管理员刚给某人改了角色，那人刷新页面就看到"404 页面未找到"。

### 修复方向

| 步骤 | 内容 |
|---|---|
| 1 | 前端单点化的"会话已失效"信号：401 统一走一个出口，清令牌 + 跳登录页 + 说明原因 |
| 2 | 守卫区分**401** 与其它加载失败：401 走会话失效出口；网络/500 仍按现状兜底（不扩大改动面） |
| 3 | 登录页能显示"会话已失效，请重新登录"，而不是无声出现 |
| 4 | 同一条提示去重，避免 4 连弹 |
| 5 | 承重测试（前端单测 + e2e）+ 门禁 + 提交（**不推送**） |

### 关键决定（动手前先定）

1. **不能靠 401 无条件跳转**。登录接口本身在口令错误时返回 401，
   登录页必须原地报错而不是被自己的 401 弹走。需要在 401 分支里
   排除登录请求本身。
2. **不改动守卫"避免死循环"的初衷**。菜单加载失败的兜底逻辑保留，
   只把 401 这一种分出来——死循环风险来自"跳登录页 → 又触发守卫"，
   而登录页在白名单里，本身不会死循环。
3. **提示去重按"同一文案短时间内只弹一次"**，不做全局错误队列。
   后者会把不同错误也压掉，而压掉错误与重复弹错一样糟。
4. **顺手补齐 `METRICS_*` 三项文档**，但不为其单独立测试——
   文档错误由 README 自身核对，造测试反而是自证。

### 下一步

步骤 1–5。仍在"全部完成或收到指令才统一推送"的约束下。

---

## v0.15.0 动手前的复核（本轮实测，2026-10-03）

### 缺陷已独立复现（不照抄上一轮记录）

用一次性脚本（`/tmp/repro-v015.mjs`，不进 `e2e/suites`）跑真实 Chrome：

```
logout -> 200
同一令牌再调 /api/auth/me -> 401 {"code":401,"message":"令牌已被注销，请重新登录"}

path            = /system/user            ← 没跳登录页
页面正文         = 404 页面未找到 返回首页 未授权，请重新登录 ×4
localStorage 令牌 = 有                      ← 令牌残留
弹窗文案         = ["未授权，请重新登录","未授权，请重新登录","未授权，请重新登录","未授权，请重新登录"]
```

四条缺陷全部重现。**但"为什么恰好是 4 条弹窗"上一轮没查**，本轮查清了，
且答案不是"请求风暴"这么含糊：

- `menuStore.load()` 与 `permissionsStore.load()` 是 `Promise.all` 并发 → **2 个 401**
- 每个 401 被 `api/index.ts` 响应拦截器 `showError` 弹 **1 次**
- 随后 `menuStore.load()` 的 catch 调 `handleError(error)`，它**又 `showError` 一次**
- `permissionsStore.load()` 同理

即 `2 请求 × 2 次弹窗 = 4`。**真正被重复弹的是"拦截器已经弹过、store 又弹一遍"**，
这是全站性的：任何一个走 `handleError` 的失败都会被弹两次，与 401 无关。
只做"401 去重"只能压住这一次的表象。

### 又发现两处同一缺陷族的问题（本轮实测得到）

**其一：401 的后端原话被前端覆盖掉了。**

```
POST /api/auth/login 口令错误
  → {"code":401,"message":"凭证错误: 用户名或密码错误"}
前端实际显示 → "未授权，请重新登录"
```

拦截器对 401 是**无条件**写死 `message = '未授权，请重新登录'`，
把后端已经区分好的三类原因（凭证错误 / 令牌被吊销 / 令牌无效过期）压成同一句。
用户在登录页输错口令，被告知"未授权，请重新登录"——**说的是另一件事**。

**其二：登出后立刻再登出会 401，而这条 401 不该触发"会话失效"提示。**

```
吊销后的令牌 POST /api/auth/logout → 200   （第一次）
同一令牌再登出          → 401
```

`logout()` 靠 `finally` 清本地会话，所以功能上没坏；但一旦 401 开始跳登录页，
用户点"退出登录"就会看到"会话已失效"——他**正是**主动结束会话的人。
`/auth/logout` 必须与 `/auth/login` 一起排除在会话失效出口之外。

### 修订后的实现决定

1. **401 单点出口**（`utils/session.ts` 新模块）：清令牌 + 清缓存 + 跳登录页 +
   把**后端给的原话**带过去。不在前端另编一套"会话失效"文案——
   后端已经分得清（`令牌已被注销` / `登录状态已失效` / `令牌无效或已过期`），
   前端编的通用句只会把这些区别重新抹平（v0.14.0 的教训：界面不说谎的前提是
   不覆盖比它更准的信息）。
2. **排除清单按"这个 401 有没有正常的用户语义"来定，不只排登录**：
   `/auth/login`（口令错）、`/auth/logout`（主动登出）排除；
   其余（会话被吊销/过期）走会话失效出口。
3. **修 `handleError` 的二次弹窗**，而不是给 401 加去重补丁。
   `handleError` 只在**错误尚未被展示过**时才弹。这样全站失败都从 2 条降到 1 条。
4. **去重仍要保留**，但理由变了：并发请求（`Promise.all`）仍会对同一原因各弹 1 次，
   `2 请求 × 1 次 = 2` 条。按"同一文案短时间只弹一次"压到 1 条。
5. **本地令牌过期也走同一出口**：守卫现在自己 `localStorage.clear()` 后跳登录页，
   是一次性的旁路实现，且**清的是整个 localStorage**（会连"记住密码"的
   用户名一起抹掉）。改为走统一出口，只清该清的键。
6. **守卫区分 401 与其它失败**：网络/500 仍按现状放行由 404 兜底。
7. **`METRICS_*` 三项补进 README**（顺手修，不单独立项）。

### 承重测试的判据

必须能抓住"漏改的那一处"，而不只是"页面上有句话"：

- 令牌被吊销后 `goto /system/user` → `pathname` 必须落在 `/login`（**反向**：不能是 `/system/user`）
- 页面正文**不得**出现 `404 页面未找到`（这是本版的核心缺陷）
- localStorage 令牌必须为无
- 401 弹窗恰好 1 条（既抓重复弹，也防止"静默不提示"）
- 登录页显示后端原话（`令牌已被注销`），不是前端通用句
- **反向判据**：口令错误时**不得**跳登录页、**不得**清令牌，
  且必须显示 `用户名或密码错误`（证明排除清单生效且没把原因盖掉）
- **反向判据**：正常登录后进 `/login` 不得出现会话失效提示（提示不是常驻的）

### 下一步

按上述修订动手。门禁全绿后提交，**不推送**。

---

## v0.15.0 收尾（本轮，2026-10-03）

### 已提交内容（未推送）

改动集中在前端会话失效链路 + README/CHANGELOG + 版本号：

- `frontend/src/utils/session.ts`（新）：401 单点出口。记原因、去重、
  `sessionStorage` 落盘、由 router 注册的回调执行跳转。不自己 import 路由
  （否则 `router → stores → api → session → router` 循环依赖）。
- `frontend/src/api/errors.ts`（新）：`ApiError`（带 `status` + `reported`）、`isUnauthorized`。
- `frontend/src/api/index.ts`：401 分诊（后端原话优先、豁免 `/auth/login` 与
  `/auth/logout`、登出 401 静默）、`ApiError` 替换裸 `new Error(message)`。
- `frontend/src/api/helper.ts`：`handleError` 遇 `reported` 不再二次弹窗。
- `frontend/src/router/index.ts`：守卫走同一出口 + `next(false)`。
- `frontend/src/views/login/index.vue`：提示条（取走即清除）。
- `frontend/src/utils/session.ts` 里的 `buildSessionEndedMessage` 防"请重新登录，请重新登录"。
- README 补 `METRICS_FLUSH_INTERVAL_SECONDS` / `METRICS_KEY_TTL_SECONDS` /
  `METRICS_MAX_BUFFERED_ENDPOINTS`；版本号 bump 至 0.15.0；CHANGELOG 写 `[0.15.0]`。

### 本轮新增（接上一轮 checkpoint）

**`frontend/src/router/__tests__/authGuard.spec.ts`**（3 条）——补上了唯一的悬案。

上一轮遗留的疑点：注入 1（守卫 401 分支改成 `next()`）**没被抓**，e2e 27 条全绿。
本轮先复现确认（改回 `next()` 跑 e2e，仍 27/27），再查清了原因：
跳转由拦截器注册的 handler 驱动（`router.replace('/login')` 立即改写 `pendingLocation`，
vue-router 随即以 `pendingLocation` 变了为由取消原导航），
所以守卫分支在当前接线方式下属于**纵深防御而非承重**。这是实测结论。

新单测直接观察守卫传了哪个 `next`，注入 1 因此变红。但仍如实记录：
**这一行今天不承重**，它防的是 handler 注册时机变化（某个入口忘了 import `router`，
`notifySessionEnded` 就静默无操作）时的 404 兜底回归。

顺带发现并修掉两个坑，都写进了注释：

1. `vitest.config.ts` 原本没注册 `@vitejs/plugin-vue`。路由表有懒加载 `.vue`，
   导航一旦成立就在 import-analysis 阶段炸（"content contains invalid JS syntax"）。
2. vue-router 对**同一目标**的第二次 `push` 直接判定 duplicated 并**跳过守卫**。
   三条用例都写 `router.push('/system/user')` 时，"过期令牌"那条会空过
   （没跑守卫，自然没触发 `notifySessionEnded`）。必须给每条不同目标路径。

### 门禁（全绿）

| 项 | 结果 |
|---|---|
| `cargo fmt --check` / `cargo clippy --all-targets -D warnings` | 零警告 |
| 后端单测 | **74** passed |
| 后端集成（`--include-ignored --test-threads=1`） | **117** passed（109 ignored + 8 非 ignored） |
| 前端 typecheck / lint | 通过（`env.d.ts` 既有 1 warning，非本轮引入） |
| 前端单测 | **133** passed / 15 files（新增 28 条） |
| 前端 build | 通过 |
| e2e 8 套件 | **160** passed（16+28+25+13+27+14+13+24） |
| 授权探针 | **41/41** passed |

Rust 侧零改动，只 bump 了 `Cargo.toml` 版本号（`Cargo.lock` 随之变一行）。

### 缺陷注入

| 注入 | 结果 |
|---|---|
| A：401 不走会话失效出口 | ✅ 被抓，6 条红 |
| B：把 `/auth/login` 移出豁免清单 | ✅ 被抓，2 条红 |
| C：`handleError` 恢复二次弹窗 | ✅ 被抓，2 条红 |
| D：守卫 401 改成 `next()` | ✅ 补守卫单测后被抓（e2e 抓不到，已记录原因） |

### 环境备忘（本轮实测）

- 后端必须用**独立 exec 会话**跑（`nohup ... &` 随 shell 退出被收走，
  下一次 `curl` 就 404）。
- 开发库是 `axum_api_manual`（不是 `axum_api`）；集成测试库是 `axum_api_test`。
  集成测试前须**先停后端**，再 `dropdb/createdb` + `redis-cli -p 56379 flushall`。
- 后端启动 env：`DATABASE_URL` / `REDIS_URL=redis://127.0.0.1:56379` /
  `JWT_SECRET`（≥32 字符）/ `RATE_LIMIT_IP_MAX=100000 RATE_LIMIT_USER_MAX=100000` /
  `MIGRATE_ON_STARTUP=true` / `CORS_ALLOWED_ORIGINS=http://localhost:3000`
- e2e 账号 `admin` / `admin123`（`E2E_USER` / `E2E_PASS` 可覆盖）。

### 当前 git 状态

- 分支 `master`，基线 `335daa48 feat(v0.14.0)`
- 本轮改动**尚未提交**（见 `git status`）
- 待推送队列：`a0ac25ce`…`335daa48` 共 6 个提交 + 本轮 v0.15.0 一个
- **用户指令：全部工作完成或收到指令才统一推送。不要推送。**

### 下一步

v0.15.0 已完结。提交后等待用户指令：继续 v0.16.0，或统一推送。

---

# v0.16.0 开始（2026-10-03）

基线：`67e5599d feat(v0.15.0)`，分支 `master`，工作区干净。
待推送队列 7 个提交（`a0ac25ce`…`67e5599d`），**未推送**（用户指令）。

## 主题：字典的三个开关都是摆设

沿 v0.10.0–v0.15.0 的做法：先真实复现，再修。字典模块自 v0.4.0 之后
没被任何版本正面处理过（856 行 Rust + 286 行 Vue），这次逐个端点用真实
HTTP 打了一遍，找到**三处管理员以为自己能控制、实际控制不了的东西**。

### 实测证据一：`status=disabled` 完全不生效

```
创建 sys_yesno，含 Y / N 两项 → 200
把 Y 改成 status=disabled    → 200（写入成功）
GET /api/dict/sys_yesno/items
  → {"label":"Y", ... "status":"disabled"}   ← 禁用项照样返回
```

字典**类型**层同样：把 `sys_yesno` 类型置 `disabled` 后，
读取端返回条数仍是 3。

`GET /api/dict/{code}/items` 是任意已登录用户可读的通用读取端点，
`DictSelect` 组件直接用它渲染下拉框。管理员在字典管理页把一项禁用，
所有业务页面的下拉框里它还在——而界面上那一栏写着"禁用"。

### 实测证据二：`is_default` 可以有任意多个

```
同一字典下连续创建 A(is_default=true)、B(is_default=true) → 均 200
GET /api/admin/dict/items → A.is_default=True, B.is_default=True
```

"默认"这个词的全部意义就是唯一。没有唯一约束、没有"取消旧的默认项"、
没有报错。

### 实测证据三：「刷新缓存」按钮刷新不了任何东西

```
redis-cli set dict:sys_yesno '[合法的陈旧数据]'      → OK
GET /api/dict/sys_yesno/items     → ['陈旧项']        ← 缓存确实生效
POST /api/admin/dict/refresh      → {"data":"缓存刷新成功"}
GET /api/dict/sys_yesno/items     → ['陈旧项']        ← 一点没变
redis-cli get dict:sys_yesno      → 仍是陈旧项
```

对照实验确认不是"缓存根本不生效"：走正常写接口新建一项后，
读端立刻返回新数据（`['Y','N','A','B','新项']`）——写路径的
`invalidate_cache` 是好的。**坏的是补救手段本身**：写路径失效失败时
（比如 Redis 抖动），管理员唯一的补救按钮点了等于没点。

而且审计里也记着它成功了：

```
POST /api/admin/dict/refresh | '刷新字典缓存，共 1 个类型'
```

审计说刷新了，实际上一个键都没动。

**根因**：`refresh_cache` 的实现是
`for dt in &data { let _ = get_dict_by_code(&dt.code).await; }`。
而 `get_dict_by_code` **先读缓存，命中就返回**。所以这个循环
只会把同一份陈旧数据读一遍再原样写回——注释写的"清除 Redis 中所有
字典缓存"从未发生。

### 三处的关系

不是三个独立 bug，是同一件事的三个面：**字典模块给了管理员"启用/禁用"、
"设默认"、"刷新缓存"三个控制，每个都在界面上正常工作，写入也都返回 200，
但对实际行为没有任何影响**。写入成功 ≠ 生效。

### 计划

1. `status` 真正生效：禁用项与禁用类型不进读取端点（`GET /api/dict/{code}/items`）
2. `is_default` 真正唯一：设置某项为默认时，同类型其余项自动取消默认；
   DB 层加部分唯一索引兜底（迁移 012）
3. 「刷新缓存」真的清：SCAN + DEL `dict:*`，然后回填；
   **返回值如实报告清了多少键**，不再无条件说"成功"
4. 承重测试：后端集成测试 + 前端单测 + e2e 套件
5. 缺陷注入：把三处修复分别破坏，确认对应用例变红
6. 全门禁 + 提交（不推送）

### 状态

实测完成，主题已定。三条探针夹具（`sys_yesno`、`probe_empty`、
脏缓存）**已全部清理**，`GET /api/admin/dict/types` 现在返回空数组。


## v0.16.0 收尾（2026-10-03）

### 先纠正上一条记录：夹具并没有清干净

上面那条"已全部清理"是**错的**。接手时实测 `axum_api_manual` 里仍有
`sys_yesno` + 4 个项（Y/N/A/B，其中 Y 是 disabled、B 是 default）。
已删除类型与项、并 flush 掉 `dict:*`，现在 `GET /api/admin/dict/types` 确实返回 `[]`。

教训：清夹具这件事不能靠上一轮的文字记录，要自己查一次库。

### 第四处缺陷（本轮新发现，不在原计划里）

写 e2e 套件时，"管理页要能看出哪些类型已禁用"这条一直红，截图一看
**整个字典管理页是空白的**——只有一条分隔线，没有卡片、没有列表。

DOM 取证：

    .dict-page          504x369   text = "字典管理 刷新缓存 新增字典"
    .n-split-pane-1     175x369   text = ""     <- 空
    .n-split-pane-2     326x369   text = ""     <- 空
    n-split 实例的 slots 键 = ["left", "right"]

根因：页面写的是 `<template #left>` / `<template #right>`，
而 naive-ui 的 `n-split` 读的是**位置**插槽 `$slots[1]` / `$slots[2]`。
上游 demo（`src/split/demos/enUS/slot.demo.vue`）用的就是 `#1` / `#2`。
写错时**不报错、不告警**，只是静默渲染成空。

这条后端接口完全正常、菜单也点得进去，所以任何只看接口的检查都发现不了。
改成 `#1` / `#2` 后立刻正常。已把原因写进代码注释。

### 一个把调试带偏的坑：跑着的二进制是旧的

e2e 里刷新缓存一直报 `{"cleared_keys":0,"reloaded_types":0}`，
而 Redis 里明明有 `dict:probe_x`、DB 里明明有 enabled 类型。

原因是注入实验用 `cp` 恢复源码后**没有重新 `cargo build`**，
跑着的 `./target/debug/axum-api` 还是注入版（返回值写死 0）。
`cargo test` 不会更新这个 bin。

教训：注入实验恢复源码后，要么重建二进制再验真实服务，要么就别在真实服务上验。
判据上也该加一条：**数字全 0 且库里有数据，本身就可疑**。

### 缺陷注入（6 处，全部验证过会红）

| 注入 | 结果 |
|---|---|
| 读取端不过滤 disabled 项 | ✅ 被抓 |
| 读取端不过滤 disabled 类型 | ✅ 被抓 |
| `create_item` 不取消旧默认项 | ⚠️ 首轮**没被抓** → 补 POST 路径用例后被抓 |
| `refresh_cache` 退回旧循环 | ✅ 两条 refresh 用例同时红 |
| `buildRefreshMessage` 退回无条件成功 | ✅ 前端单测红 |
| `#1`/`#2` 退回 `#left`/`#right` | ✅ 3 条空白页守卫红，其余 21 条仍通过 |

第三条那一轮同时暴露了一个真缺口：原有默认项用例只走 PUT，
POST（create）路径**零覆盖**。补用例后又发现唯一索引冲突会变成
500「服务器内部错误」，于是加了 `map_dict_item_write_violation` → 409。

### 承重测试

- 后端集成：6 条字典用例（`tests/api_integration.rs`）
- 前端单测：4 条（`frontend/src/utils/__tests__/dict.spec.ts`）
- e2e：`e2e/suites/v016-dict-controls-work.mjs`，24 条断言全绿
  - 关键一条是**打开真实下拉框**读渲染出来的选项文字，
    而不是只读接口返回值——"界面里还有那一项"才是原本的缺陷

### 附带修掉：导出 Excel 会 500（既有问题，跑全量 e2e 才暴露）

`v013` 套件在全量跑时红了：`GET /api/admin/logs/audit/export` 返回 500。
后端是 panic，不是业务错误：

    thread 'tokio-rt-worker' panicked at rust_xlsxwriter-0.82.0/src/xmlwriter.rs:291:
    byte index 28 is not a char boundary; it is inside '（' (bytes 27..30)
    of `新建用户 "prbo6hpb_x20"（67d7a0e0-...），角色：prbo6hpb_strong`

定位过程：

1. `_x` 在第 22 字节，库按字节切 `original[24..28]` = `20"\xef`
2. 第 28 字节落在 `（`（27..30）中间 → panic
3. 触发条件极容易凑齐：审计摘要本来就有全角括号，
   用户名又是管理员自己填的 → **任何含 `_x` 的名字都让整个审计导出永久 500**

为什么以前没发现：这是既有问题（`src/controller/user.rs` 的摘要格式早于本版），
但只有**全量按序**跑 e2e 才会撞上——`role-assignment-guard` 先造出 `*_x20` 用户，
后面的 `v013` 再导出才炸。单跑 v013 或按别的顺序跑都看不到。

修法：`rust_xlsxwriter` 0.82 → 0.99.1。本项目只用到 `Workbook` / `Worksheet` /
`Format` / `write_string` / `set_column_width` 这几个稳定 API，**升级零代码改动**。
验证过导出内容逐字未变（`删除字典类型 "probe_x"（0c292627-...）` 原样出现在
sharedStrings 里，没被转义成 `probe_x005F_x`）。

顺带删掉 e2e/probe 留在 `axum_api_manual` 里的 `prb%` 夹具
（2 个用户、6 个角色）。正是这些 `_x` 用户制造了上面那个 panic 的输入。
清理后：1 用户（admin）/ 2 角色 / 0 字典类型。

### 门禁（全绿）

| 项 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | ✅ 零警告 |
| 后端单测 | ✅ 74 passed |
| 集成（非 ignored 组） | ✅ 8 passed |
| 集成（`--ignored` 组，真实 PG+Redis） | ✅ 116 passed |
| 前端 lint | ✅ 0 error（1 个既有 warning 在 `env.d.ts`，与本版无关） |
| 前端 typecheck | ✅ |
| 前端 vitest | ✅ 137 passed / 16 文件 |
| 前端 build | ✅ |
| e2e 9 套件 | ✅ 全绿（`v013` 由 18/21 → 25/25） |
| 授权探针 | ✅ 41/41 |

版本号已 bump 到 0.16.0（`Cargo.toml` + `frontend/package.json` + `Cargo.lock`）；
CHANGELOG 新增 `[0.16.0]`；README 第 62 行字典能力描述改为如实描述。

注意：bump 版本号会让 `Cargo.lock` 里的 `axum-api` 版本过期，
`cargo clippy --locked` 会直接报错。需要 `cargo update --offline -p axum-api`。

### 尚未提交

全部改动仍在工作区，**未提交、未推送**（按用户指令：全部完成或收到指令才统一推送）。

## v0.17.0 缺口分析（2026-10-03，本轮只读分析，未动代码）

### 起始 git 状态

- 分支 `master`，HEAD = `472d60c1 feat(v0.16.0): 字典的三个开关都是摆设`
- 工作区干净。v0.16.0 已发布：master 已推送，
  v0.10.0–v0.16.0 七个 annotated tag + GitHub Release 全部创建并推送完毕。
- 本轮唯一写操作：往本文件追加本节，以及清理 dev 库探针夹具。

### 主题：菜单树能成环 → 整棵子树静默消失 → 递归查询永不收敛 → 全站瘫

与 v0.10–v0.16 同一根因族：**写入返回 200，但对实际行为毫无影响，甚至有害。**

上一轮 checkpoint 只测到"成环后菜单静默消失"。本轮把它往下挖了两层，
破坏等级从"一个功能坏"升级为"整个服务不可用且界面无法自救"。

### 根因（代码位置）

`src/repository/menu.rs:286`

```rust
let parent_id = fields.parent_id.or(menu.parent_id);
```

`update()` 对 `parent_id` **不做任何校验**，直接进 `UPDATE menus SET parent_id=$2`：

- 不校验 `parent_id != 自身 id`（自引用）
- 不做环检测（把父节点挂到自己的子孙下面）
- 控制器层 `src/controller/menu.rs:181 update_menu` 只防了 `permission` 改写旁路，
  `parent_id` 一路裸奔到 SQL
- 数据库 `menus_parent_id_fkey` 只能挡住"指向不存在的菜单"（所以**孤儿节点不可能出现**，
  上一轮那个孤儿假设已被 FK 证伪），**挡不住环**

`create()` 路径同样直接 `.bind(menu.parent_id)`，无校验。

### 破坏链（三级，全部实测）

**第一级：菜单及其整棵子树静默消失，且管理页看不见它**

`build_tree`（`menu.rs:535`）判定根节点看"父节点是否在集合内"。
成环后环上每个节点的父都在集合内，于是**没有一个被判定为根 → 整支被剪掉**。
不报错、不超时，只是消失。

实测（真实种子菜单「用户管理」`7f000000-...-0007`，父「系统管理」`...-0006`）：

```
PUT /api/admin/menus/{用户管理}  parent_id={用户管理}   -> 200
修改前侧栏含「用户管理」: True    修改后: False
GET /api/admin/menus 侧栏字节数 4531 -> 4201
```

关键：`GET /api/auth/menus`（侧栏，所有用户）和 `GET /api/admin/menus`（管理页数据源）
**是同一棵树、同一个 `build_tree`**，所以菜单在管理页**也消失**。
管理员在界面上看不到它 → 也就无法点开改回来 → 只能直连数据库救。
审计把这次操作记为一次**正常成功**的更新。

**第二级：删除守卫永久挂起**

`granted_codes_in_subtree`（`menu.rs:404`）用 `WITH RECURSIVE ... UNION ALL` 走子树。
`UNION ALL` 不去重 → **环上永不收敛**。实测（`statement_timeout='5s'` 兜底才停下来）：

```
ERROR:  canceling statement due to statement timeout
```

实际服务路径 `DELETE /api/admin/menus/{环上任意节点}`
（经 `controller/menu.rs:326` 调用）：客户端 10s 超时，**无响应，连接不归还**。

也就是说：环一旦形成，**连"删掉它"这条自救路也同时被堵死**。

**第三级：连接池被占满 → 整个 API 服务不可用**

连接池上限默认 20（`src/config/mod.rs:168` `DB_POOL_MAX_SIZE`，无默认覆盖），
代码里**没有 `statement_timeout`**。构造 28 节点大环后并发 22 个 DELETE：

```
19 个请求挂起（客户端 25s 超时，http=000）
 3 个请求 500（等不到连接，10s acquire_timeout）
库内状态：active x21，最长已跑 41s 且持续增长
```

此时一个与菜单**毫无关系**的端点：

```
GET /api/admin/users -> 500 (10.003s)   # 池被占满，acquire 超时
```

**这不是菜单功能坏了，是全站 500。** 且恢复手段只有两条：
直连 DB 手改，或重启后端——而界面上一个入口都没有。

### 同族第二处缺陷（前端，静默改结构）

`frontend/src/views/system/menu/index.vue`

- `openEdit(id)`（约 L160）只设 `isEditing/editingId/showModal`，**不重置 `parentId`**
- `handleSubmit()`（约 L178）统一发 `parent_id: parentId.value || undefined`
- 弹窗模板（L21–L54）**根本没有父级选择字段**，管理员无从设置、也无从察觉

后果：先点某个节点的「新增子菜单」（`openCreate(pid)` 把 `parentId` 设成该节点），
再点任意节点的「编辑」并保存 → 被编辑的菜单**被静默挂到上一次那个父节点下**，
弹窗里没有任何东西提示这件事。菜单层级在管理员不知情的情况下被改了。

（这解释了为什么第一级缺陷必须用 curl 复现：走 UI 编辑路径不会带上 `parent_id`，
是上面这条"脏状态泄漏"路径才会。）

### 附带纠正一个错误判断（写下来免得下次再踩）

"菜单在树上找不到 ⇒ 已删除"是**错的**。成环节点在树上永远不可见，
所以上轮以为已删净的 A/B 探针其实还留在库里（本轮查库才发现，
`DELETE` 因第二级挂起根本没生效）。删除是否成功**只能查 DB 确认**。

### 本轮探针清理结果

- 删 `池探针*` 26 条、`环探针*` 2 条、上一轮遗留 A/B 2 条、`prb*` 4 条
- dev 库 `axum_api_manual` 复位到 42 菜单，悬空父 0，环 0，可达节点 42/42
- 挂起的后端连接已 `pg_terminate_backend` 清场，`GET /api/admin/users` 恢复 200 (0.007s)
- 注：`prb%` 那 4 条按钮型菜单是**更早的 e2e/probe**留下的，不属本轮；本轮一并清掉

### 下一步

按用户指令**只出计划、不实现**。v0.17.0 计划见本会话回复，
核心修复面：`update`/`create` 双路径加 parent_id 校验（存在 + 非自身 + 不在自身子树内，
把现有递归 CTE 改成 `UNION` 或加环检测终止条件）+ 前端补父级字段并重置脏状态 +
不可达菜单要能被看见和修复。

### 同族第三处缺陷（后端，写入成功但毫无影响）

`UpdateMenuRequest.parent_id: Option<Uuid>`（`src/model/menu.rs:100`）里
**「没传这个字段」与「显式传 `null`」不可区分**，而 `update()` 用的是
`fields.parent_id.or(menu.parent_id)`（`menu.rs:286`）。

实测（把「用户管理」摘成根菜单）：

```
PUT /api/admin/menus/{用户管理}  {"parent_id": null}  -> 200
返回体里的 parent_id = 7f000000-...-0006   # 仍是「系统管理」
库里真实 parent_id   = 7f000000-...-0006   # 没变
```

即：**无法通过 API 把任何菜单摘成根菜单**——请求成功、返回 200、
字段原样回显，但结构纹丝不动，没有任何提示说明它被忽略了。

而前端弹窗又没有父级字段（见上），于是当前 UI + API 组合下
**管理员根本没有任何途径调整菜单层级**，只能新建。

## v0.17.0 动手前的计划（2026-10-03）

### 起始 git 状态

- 分支 `master`，HEAD = `472d60c1 feat(v0.16.0): 字典的三个开关都是摆设`
- 工作区仅 `docs/AI_HANDOFF.md` 被修改（上一节的分析记录）
- 版本号 0.16.0。**本版完成后 bump 到 0.17.0**，
  bump 后必须 `cargo update --offline -p axum-api`，否则 `cargo clippy --locked` 报错

### 修复面（4 块，动手前先定死，避免实现期漂移）

| # | 改动 | 文件 |
|---|---|---|
| 1 | `parent_id` 三态化：区分「没传」/「显式 null=摘成根」/「指定父级」 | `src/model/menu.rs` |
| 2 | 挂载校验：父级必须存在、非自身、不在自己子树内（Rust 侧向上走 + visited 集合） | `src/repository/menu.rs` |
| 3 | 递归 CTE `UNION ALL` → `UNION`：库里已有环时 DELETE 也不会挂起 | `src/repository/menu.rs` |
| 4 | 结构诊断 + 自救：`GET /api/admin/menus/diagnostics` + 前端告警条与一键摘成根 | `src/repository/menu.rs`、`src/controller/menu.rs`、`src/router/mod.rs`、`frontend/src/api/menu.ts`、`frontend/src/views/system/menu/index.vue` |

### 关键决定

1. **环检测放在 Rust 侧，不放 SQL 递归**
   向上沿 `parent_id` 走，用 `HashSet` 记 visited。这样做的理由：
   库里**可能已经存在环**（历史脏数据 / 直连 DB 写入），
   任何无 visited 的向上遍历自己就会死循环——用 SQL 递归同样躲不掉。
   顺带把「父级不存在」也一起给出可操作消息，而不是让 FK 抛 500。

2. **自救复用 `PUT /admin/menus/:id` + `parent_id: null`，不新增写接口**
   第 1 项做完，「摘成根」这条最朴素的修复动作就自动可用了。
   diagnostics 只负责**把坏节点显出来**，修复仍走既有公开 API，
   避免多出一条只有诊断页在用、却同样有权限需求的写路径。

3. **不改 `build_tree` 的「父不在集合内即视为根」语义**
   那条语义在按角色过滤时是**必需且正确**的（勾了子菜单没勾上级目录的角色
   需要看到子菜单）。把坏节点显出来只能另开诊断口，不能改树本身的构造。

4. 前端父级下拉**排除自身与自身子树**，但这只是减少误操作；
   真正拦住环的是后端校验，不依赖前端。

### 承重验证（证明修好了，不是证明没坏）

- 集成测试（`--ignored`，真实 PG）：自引用被拒 / 挂到孙节点被拒 / 挂到不存在的父被拒 /
  合法改父成功 / `parent_id: null` 真的摘成根 / 诊断口能报出人为造的环
- 回归：既有的 `an_invalid_menu_type_is_a_bad_request`、
  `deleting_a_menu_removes_its_nested_permission_buttons` 等不得变红
- 缺陷注入：逐个注释掉校验点，确认对应测试变红
- CTE 终止性：库里造环后直接跑 DELETE，必须有响应（不再挂起）
- 前端单测 + e2e 套件

## v0.17.0 实施记录（2026-10-03，已完成，门禁全绿）

### 起始 git 状态

- `master`，HEAD = `472d60c1 feat(v0.16.0): 字典的三个开关都是摆设`
- 工作区只有 `docs/AI_HANDOFF.md`（本轮分析记录）
- 版本 0.16.0 → **已 bump 到 0.17.0**（`Cargo.toml` / `frontend/package.json` / `Cargo.lock`，
  bump 后跑了 `cargo update --offline -p axum-api`，故 `cargo clippy --locked` 通过）

### 实际改了哪些文件

| 文件 | 改动 |
|---|---|
| `src/model/menu.rs` | `UpdateMenuRequest.parent_id` → `Option<Option<Uuid>>` + `double_option` 反序列化器；新增 `UnreachableMenu`；新增三态单测 |
| `src/repository/menu.rs` | 新增 `ensure_attachable`（环检测，Rust 侧向上走 + visited）、`find_unreachable`；`create`/`update` 接上校验；递归 CTE `UNION ALL` → `UNION` |
| `src/controller/menu.rs` | 新增 `GET /api/admin/menus/diagnostics`；`update_menu` 审计补记移动/摘成根 |
| `src/router/mod.rs` | 注册 `/api/admin/menus/diagnostics`（静态段优先于 `{id}`） |
| `src/docs/mod.rs` | 新路由登记进 OpenAPI 契约 |
| `frontend/src/utils/menu.ts` | 新增 `collectSubtreeIds` / `buildParentOptions` |
| `frontend/src/api/menu.ts` | `parent_id?: string \| null`；新增 `UpdateMenuReq`、`UnreachableMenu`、`diagnostics()` |
| `frontend/src/views/system/menu/index.vue` | 补"上级菜单"字段；`openEdit` 回填并重置父级；诊断告警条 + 一键摘成根 |
| `tests/api_integration.rs` | +9 个 v017 用例；`cleanup_temp_menu_dir` 的 CTE 改 `UNION`；新增 `flatten_menus_to_roots` |
| `frontend/src/__tests__/menuParentOptions.spec.ts` | 新增，7 条断言 |
| `e2e/suites/v017-menu-hierarchy.mjs` | 新增套件，26 条断言 |
| `e2e/suites/v014-retention-honesty.mjs` | 修断言假红（见下） |
| `CHANGELOG.md` / `README.md` | 版本条目与能力描述 |

### 门禁（本地，全绿）

| 项 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | ✅ 零警告 |
| 后端单测 | ✅ 75（+1） |
| 集成（非 ignored 组） | ✅ 8 |
| 集成（`--ignored` 组，真实 PG+Redis） | ✅ 125（116 → +9） |
| 前端 lint | ✅ 0 error（1 个既有 warning 在 `env.d.ts`，与本版无关） |
| 前端 typecheck | ✅ |
| 前端 vitest | ✅ 144 / 17 文件（137 → +7） |
| 前端 build | ✅ |
| e2e | ✅ 10/10 套件（新增 v017 26/26） |
| 授权探针 | ✅ 41/41 |

### 缺陷注入（5 处，每处都有对应用例变红）

1. CTE 改回 `UNION ALL` → `deleting_a_menu_does_not_hang...` 红，耗时 15.5s（超时兜底生效）
2. `ensure_attachable` 直接 `return Ok(())` → 自引用/子孙/不存在上级三条红，正向用例仍绿
3. 还原 `.or()` 语义 → 摘成根红，**连带把 diagnostics 的自救路径也测红了**
   （证明"救回来"确实依赖"摘成根真的生效"，不是空转）
4. 前端去掉自身子树过滤 → 4 条前端单测红
5. 前端还原 `openEdit` 不重置父级 → e2e 红，并明确报出 `实际="v17hohnz5_A"`（B 被挂到 A 下）

### 过程中被自己绊倒的三处（都记下来，免得下次重犯）

1. **"树上找不到 = 已删除"是错的**。成环节点在树上永远不可见，
   所以上一轮以为删净的 A/B 探针其实还在库里。删除是否成功**只能查 DB 确认**。
2. **测试清理助手自己会挂死**。`cleanup_temp_menu_dir` 内部也是 `UNION ALL` 递归：
   一旦某个用例失败跳过清理，残留环会让后续清理永久挂起。已改 `UNION`，
   并加 `flatten_menus_to_roots` 在清理前剪环。另有一个用例把叶子摘成根后
   它就不再随父级联删除了，漏了单独清理，每次跑都留一条孤儿菜单。
3. **e2e 里 `const name = inputs[0]` 会抛 Illegal invocation**：
   局部变量 `name` 遮蔽了全局 `window.name`，原生 setter 以它为 `this` 就炸。
   另外 naive-ui 的行容器与内层文字 span 类名互为前缀
   （`n-tree-node-content` / `n-tree-node-content__text`），
   朴素选择器会同时命中两层；下拉选项必须在 `.n-tree-select-menu` 内取，
   否则取到的是弹窗背后整棵页面树，断言会退化成恒真。

### 顺带修掉的既有缺陷

- **v0.14 e2e 套件是状态依赖的假红**：`v014-retention-honesty.mjs` 用
  "alert 里含'早于'两个字"判定范围提示，但**清理横幅**在跑过清理任务后也含"早于"。
  v0.16.0 门禁时绿（库里还没有清理记录），本轮一跑就红。
  **已用 `git stash` 在干净的 v0.16.0 上复现确认**：不是本版引入的回归。
  改为锚定范围提示自己的文案。
- **测试自身的 CTE 挂起隐患**（见上）。

### 环境备注

- QQ 进程占着 8080；后端能起来是因为抢在它前面 bind。e2e 需要后端在 8080。
- vite 只监听 `[::1]:3000`（IPv6），`curl 127.0.0.1:3000` 连不上，
  `e2e/lib/harness.mjs` 已用 `localhost` 兼容。
- 后台进程随 exec 会话回收，e2e 期间需用 PTY 会话保持 vite 与后端存活。

### 下一步

**尚未提交**。按用户既有指令：本地提交、不推送，等全部工作完成或收到指令再统一推送。

---

## v0.17.0 收尾：核对探针残留时，发现工具自己在撒谎

**起始 git 状态**：分支 `master`，HEAD = `472d60c1 feat(v0.16.0): 字典的三个开关都是摆设`，
工作区有 v0.17.0 的 13 个改动 + 1 个新文件，**全部未提交**。

**任务**：上一轮遗留的唯一未收尾项——探针报告"夹具已全部清理"，
但 `axum_api_test` 库里躺着 6 条 `prbfb7fc_*` 残留。
按上一轮的判断"若是探针自身缺陷，顺手修掉并说明"。

### 结论：是探针的缺陷，而且是三处叠加的假绿

| # | 缺陷 | 为什么是假绿 |
|---|---|---|
| 1 | 清理顺序直接删角色/菜单 | 探针自造的 `probe:<uniq>:<n>` 是全新权限码，admin 按设计不持有它（种子"只授权新建行"）。删除撞上 v0.8.0/v0.9.0 的授权下界必然 403，探针照旧报"清理完成" |
| 2 | 清理断言写反 | `created.*.length > 0` 判的是"我建过东西"，标签写着"已全部清理" |
| 3 | 数据侧断言比一个从不存在的人 | body 用 `ctx.name()`、verify 查 `ctx.pending()`，两个不同名字 → 恒真 |

第 3 条最要命：它正是探针最核心的断言（建号时的授权天花板）。

**关键实测**：探针自造的动态权限码**不是产品死角**。出路存在且已验证——
`PUT /roles/{id}/menus` 传 `{"menu_ids":[]}` 收回授权（**降权方向不设这道限**），
再删菜单、删角色即 200。全程真实 HTTP。

### 改了什么

| 文件 | 内容 |
|---|---|
| `e2e/probe-write-guards.mjs` | 清理改"删用户 → 收回授权 → 删角色 → 删菜单"；断言改为**按名字回查数据库**（新增 `findResidue`，翻页扫全 + 树 + diagnostics）；`POST /api/admin/users` 的 body 与 verify 共用同一个名字；新增 `namedUsers` 账本 + `userIdByName`；删除死代码 `ctx.pending()` / `pendingNames` |
| `tests/api_integration.rs` | `granted_temp_button` 返回值 `TempCodeFixture`（元组 → 具名结构体，补上此前根本没返回的 `holder_role`）；新增 `cleanup_holder`；6 个调用点各自清理；新增守卫用例 `the_permission_code_fixtures_leave_no_holder_behind` |

`findResidue` 特意同时查树**和** `/menus/diagnostics`：成环节点在树上永远不可见，
只查树会把"还在"报成"没了"——那正是本版刚踩过的坑。

### 门禁（本地，全绿）

| 项 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | ✅ 零警告 |
| 后端单测 | ✅ 75 |
| 集成（非 ignored 组） | ✅ 8 |
| 集成（`--ignored` 组） | ✅ 126（125 → +1 守卫用例） |
| 前端 lint / typecheck / vitest / build | ✅ 0 error · ✅ · ✅ 144 · ✅ |
| e2e | ✅ 10/10 套件（v017 26/26） |
| 授权探针 | ✅ 41/41，且"无残留" |

**注**：`cargo test --ignored --test-threads=4` 有一条
`an_empty_keyword_means_no_filtering` 红（total 156 vs 155）。
它连着发两次查询比对 total，并行时别的用例在增删用户就会飘——
**不是回归**。单独跑绿，`--test-threads=1` 跑全量 126 条也绿。
门禁一律串行跑。

### 缺陷注入（本轮新增 3 处，都验证会红）

1. 探针去掉"先收回授权" → `探针夹具已全部清理` 红，并精确列出 4 条残留的角色/菜单/码
2. 探针不把放行侧账号登记进 `namedUsers` → 红，报出 `用户 prbdb7na_x18 | 角色 prbdb7na_strong`
3. 集成测试去掉一处 `cleanup_holder` → 该用例仍绿（**这才是问题**），
   但新增的守卫用例红，并报出 `角色 tmp_holder_role_3d1e537d` / `账号 tmp_holder_user_26093a23`

第 3 条同时说明了两件事：`cleanup_holder` 是承重的；以及"测试全绿"并不代表
"库是干净的"，所以才需要那条独立守卫。

### 刻意没做的事（留给后续版本）

**`operator_with_codes` 造的 20 处操作员角色同样没人清理**，累计已
**156 个角色 / 264 个账号**堆在 `axum_api_test` 里（每轮 +若干）。
它返回了 `role_id`/`user_id`，所以清理是可行的，只是 20 个调用点没人调。

- 这不是 v0.17.0 引入的（该函数本轮完全没碰）
- 因此守卫用例的范围**只圈 `granted_temp_button` 这一支**：
  范围一旦放大到它，这条守卫就会长期红——**一个长期红的守卫等于没有守卫**，
  不如先守住已经修干净的那一半
- 下一步该做的是给 `operator_with_codes` 配 `cleanup_operator` 并在 20 处调用，
  然后才把守卫范围放开

### 踩过的坑

- 用 shell 函数包 `curl` 时 `${3:+...}` 的引号会被吃掉，`PUT` 静默返回 400 空 body，
  差点误判成"收回授权这条路也不通"。改用裸 `curl` 重试才看清。
- Python heredoc 里写 Rust 源码要写 `\` 才能在 Rust 里留下 `\`，
  否则 `\_` 变成 `\_` 被 Rust 判成 unknown character escape。

### 下一步

**尚未提交**。按用户既有指令：本地提交、不推送，等全部工作完成或收到指令再统一推送。

---

## v0.18.0 启动：v0.11.0 收紧了口令策略，创建账号的两个入口还停在旧规则

**起始 git 状态**：分支 `master`，HEAD = `1a2772de feat(v0.17.0): 菜单树能成环，然后整个服务就没了`，
工作区干净，已与 `origin/master` 同步。

### 怎么找到的

v0.17.0 之后按"哪些模块从没被正面处理过"扫：
`dict` 有 118 次测试触及、`user` 502 次、`role` 464 次，而**注册/登录这条公开路径
在 e2e 里一个套件都没有**。顺着它摸到一条更硬的东西。

### 缺口表（全部真实 HTTP 实测，非读代码推断）

| 缺口 | 实测 |
|---|---|
| 注册页教人填一个后端不会收的密码 | 页面 placeholder 与规则都写"至少 6 个字符"（`min: 6`），后端要 **8 位 + 至少两类字符**。实测 `abcdefgh` 前端放行 → 后端 `400 密码复杂度不足`。用户填完整个表单才拿到一句原始服务端报错 |
| 管理员建号对话框同款 | `frontend/src/views/system/user/index.vue:161` 同样 `min: 6`、无复杂度规则。实测 `abcdefgh` → `400 密码复杂度不足` |
| 用户名字符集前端完全没有校验 | 后端只许 字母/数字/`_`/`-`；两处表单**都只校验长度**。实测 `user@name` 前端放行 → 后端 `400 用户名只能包含字母、数字、下划线和连字符` |
| 占位文案比后端窄 | 注册页 placeholder"3-50 个字符，字母或数字"，后端还许 `_` 和 `-`，用户想不到要用 |
| 正确的共享校验器只有 1/4 页面在用 | `utils/password.ts::passwordIssues` **只被改密页引用**；两个创建页各写各的 `min: 6` |
| 契约测试护住了工具，却没护住页面 | `password_policy_agrees_with_the_frontend_copy` 只比对 password.ts 与后端。**没有任何东西把"页面"绑到 password.ts** —— 这正是它能整整漂移七版的原因 |

### 时间线（说清它为什么是"化石"而不是"写错"）

```
7ab8e257  第三阶段  建登录/注册页，规则 min:6  ← 当时后端策略确实是 min:6
3931a5c6  第四阶段  建账号管理页，规则 min:6  ← 同上
7d6898ff  v0.11.0  策略收紧为 8位+两类字符，建 utils/password.ts + 契约测试
                  **只迁移了改密页**，两个创建页原地不动
```

`git show 7d6898ff~1:src/utils/validation.rs` 可验证 v0.11.0 之前正是
`if password.len() < 6`。所以 `min: 6` 不是随手编的，是**旧策略的化石**。

### 一条被自己推翻的假设（记下来，别重犯）

一开始怀疑登录页的 `min: 6` 会把存量弱口令用户**锁死在门外**——
`handleLogin` 里 `formRef.validate()` 失败确实会 `return`，而后端又有一条
`password_policy_is_not_applied_to_login_verification` 明说不能把策略挂到登录路径。
查了策略历史才发现：旧策略恰好就是 min 6，前端这条规则**方向上是宽松的**，
不存在锁死。差点把一个不存在的漏洞写进 CHANGELOG。
**教训**：先查规则的历史，再判定"前端更严 = 锁死"。

### 已排除的怀疑（避免下版重复劳动）

- 公开注册无法提权：`role` / `roles` / `role_id` 三个字段实测全被忽略，恒为 `user`
- 自注册账号（零权限码）扫全部 50 个端点：41×403、仅 `logout`/`health` 放行
- 转发头伪造绕过限流：`TRUST_PROXY_HEADERS` 默认 `false`，`client_ip.rs` 只在显式
  开启时采信 `X-Forwarded-For`——设计是对的
- 角色名/字典各字段：前端 max 与 DB 列宽逐项对齐，无漂移。漂移**只在账号创建路径**

### 当前计划

| 步骤 | 内容 | 状态 |
|---|---|---|
| 1 | `frontend/src/utils/accountRules.ts`：用户名/邮箱/口令/登录标识符四组规则 | ✅ |
| 2 | 注册页改用它 | ✅ |
| 3 | 管理员建号对话框改用它（并补上原先完全没有的 `maxlength`） | ✅ |
| 4 | 登录页改用它（去掉陈旧 `min:6`；标识符上限 50 → 255） | ✅ |
| 5 | 后端集成测试：弱口令/超长口令/长邮箱三类存量用户仍能经 API 登录 | ✅ |
| 6 | 契约测试：用户名样例表对账 + 断言三个页面用共享校验器 + 常量对账 | ✅ |
| 7 | 前端 vitest（11 条）+ e2e 套件（19 条） | ✅ |
| 8 | 门禁 + CHANGELOG/README/版本号 | ⬜ |

### 环境备注

- 后端在 8080（PTY）、vite 在 `[::1]:3000`、PG 55432、Redis 56379，均在跑
- 本轮探测在 `axum_api_test` 留下了一批账号（`flood_*` / `probe_reg_*` / `pw_*` /
  `admin_ui_*` / `ok_*` / `用户名字符集`），跑门禁前要清掉

### 实现过程中新发现的缺陷（都不在原计划里）

**1. 对外 OpenAPI 仍在教人用 6 位口令**（`src/model/user.rs`）

`RegisterRequest` 的字段注释被 `utoipa` 导出成对外 API 文档，而它写着
"密码（至少 6 个字符）"。v0.11.0 收紧策略后**这条注释没跟着改**，
于是 Swagger 页至今告诉调用方"6 个字符就够了"，按它写出来的调用方
会被真实接口以 400 拒掉。实测 `GET /api/openapi.json` 确认它真的在响应里。

**一份对外文档里的过期规则，和页面上的 `min: 6` 是同一种缺陷。**
顺手把 username/email 两条注释也补全了（原来只写"3-50 个字符"，
读起来像任意字符都能用）。

**2. 登录页 `max: 50` 是真缺陷，不只是"陈旧规则"**

这个字段收的是"用户名**或邮箱**"，后端 `find_by_username_or_email` 是
一条裸的 `WHERE username = $1 OR email = $1`，没有任何长度校验。
而 `users.email` 是 `varchar(255)`。实测造了个 73 字符邮箱的账号：
注册 200、API 登录 200，**但登录页 `maxlength=50` 让它根本敲不进第 51 个字符**。
持有长邮箱的合法用户在自己的登录页上登不进去。

同时**不能**把 `usernameRules` 套到这个字段上：邮箱含 `@`，会被字符集规则拒掉。

### 两个测试设计上的坑（都会造成假绿）

**A. 只断言"提示文案"等于没断言。** 邮箱上限测试最初写成
`expect(emailIssues(long).join()).toContain('255')`，而提示里的 "255" 是
**写死的常量**——把判据偷偷改成 50 之后文案照样是 "最长 255 个字符"，
注入 `email.length > 50` 时**全绿**。改成卡阈值两侧（恰好 255 通过、
超 1 个被拒）才真的会红。

**B. 样例表里的取值必须能区分被测的那把尺子。** `validate_username`
从按字节改成按字符之后，往样例表加了两条**多字节**用户名，契约测试却照样绿——
因为 `用户名`(9 字节/3 字符)、`Ωμέγα`(10 字节/5 字符)都短得两种算法结论相同。
补上 **17 个汉字（51 字节/17 字符）** 才分得开：
按字节判超长被拒、按字符判合法放行，两侧结论就此分叉。

顺带补了 30 个星平面字母 `𝒜`（30 码点/60 码元）来区分"码点 vs UTF-16 码元"——
这是 emoji 测不了的（🔒 属 So，字符集那一关就先拒了，走不到长度规则）。

**教训：断言必须落在结论会改变的地方，而不是落在常量本身。**

### 一个假阳性断言（差点让缺陷看起来已被覆盖）

e2e 里"73 字符邮箱能通过登录表单登录"这条，**在缺陷存在时依然 PASS**。
根因：harness 的 `setInput` 用原生 value setter 赋值，**绕过 `maxlength`**。实测：

| 方式 | max=255 | max=50 |
|---|---|---|
| `setInput`（setter 赋值） | 73 | **73** |
| `Input.insertText` | 73 | 50 |
| 逐字 `dispatchKeyEvent` | 73 | 50 |

给 harness 加了 `typeInto`（走 `Input.insertText`，受 maxlength 约束），
e2e 改用它，并断言"敲完之后输入框里真的有 73 个字符"。

**假阳性比没有断言更糟**——它会让一个真实缺陷看起来已被覆盖。

### 一个跨全部 11 个套件的基础设施缺陷

做上面那次注入时套件**挂死了**，只能 Ctrl-C。根因：

`waitFor` 是**故意**超时就抛错的（`cdp.mjs` 注释写了原因：继续跑会让后续
断言全建立在一个根本没出现的页面上，整轮结论都是假的）。可一旦抛错，
套件末尾的 `s.stop()` 就永远执行不到——CDP 的 WebSocket 仍然开着，
Node 事件循环因此永不退出。

而 `run.mjs` 用的是**同步** spawn，所以**一个失败的套件会让整轮 e2e 永久卡住**。
缺陷注入时尤其致命：本该看到一条 FAIL，实际看到的是永远转圈的终端。

两道防线（11 个套件全都没有 try/finally，逐个改容易漏）：
1. `harness.mjs` 装退出守卫：`uncaughtException` / `unhandledRejection`
   先关 CDP 再退出，另加一道 2 秒兜底 `process.exit`
2. `run.mjs` 给每个套件加硬超时（默认 300s，可由 `E2E_SUITE_TIMEOUT_MS` 覆盖），
   `SIGKILL` 强杀

注入验证：上限退回 50 后，套件 43 秒报 2 条 FAIL 并干净退出；
挂死探针被 runner 在 8 秒准时杀掉。

### 踩过的坑

- **后端跑的是旧二进制**。第一次跑 e2e 时"多字节用户名注册成功"红，
  一度以为修复无效——实际是 16:28 启动的进程一直没重启，
  而二进制 18:10 就重建好了。**e2e 报错前先确认后端是不是当前代码**。
- `pkill -f chrome-profile` 会把复用中的 Chrome 一起杀掉；
  harness 的 `stop()` 只关自己拉起的浏览器（复用开发者已开的那个），
  所以别图省事用 pkill 收尾。
- 本机 8080 上还占着一个 QQ 进程，起后端前先 `lsof` 确认端口。
- 后端重启需要 `JWT_SECRET`（>=32 字符），仓里只有 `.env.example`。
  本次用 `openssl rand -base64 48` 生成，存在 `/tmp/axum_jwt_secret.txt`
  ——**`/tmp` 会被清**，重启前若文件没了需重新生成（换了密钥会让旧令牌失效，
  但 e2e 每次都重新登录，不影响）。

---

## v0.18.0 里程碑：角色分页守卫从"靠运气"改成"自证"

### 起因

上一轮跑完整 `--ignored` 集成组暴露两条红，且在干净 HEAD 上用 `git stash`
复现过，确认与本版改动无关：

- `role_list_pages_over_the_same_set_as_one_big_page` —— 原断言 `total == items.len()`，
  但 `page_size` 上限就是 200，而测试库已累积 220+ 角色
- `updating_a_role_returns_the_real_row` —— 写死 `page_size=200` 取首页找不到新建角色
  （排序 `created_at ASC`，新角色在末尾）

改成真正的分页不变量 `items.len() == min(total, page_size)`，
并新增 `all_role_ids()`（按页长翻页取全量）与 `find_role_row_by_id()`（逐页查找）
两个助手，两条测试已绿。

### 但守卫本身还是假阳性（本次真正修掉的）

注入时发现：去掉 `ORDER BY r.id ASC` 后，**只有当我手工把 220 条角色的
`created_at` 批量改成同值，测试才会红**。

也就是说这条"防分页不稳定排序"的守卫，能不能变红完全取决于
"库里现有数据的 `created_at` 恰好不并列"——那是运气，不是断言。
更糟的是：一旦断言红，`created_at=2020` 的脏数据会排到全表最前面，
持续污染后续所有用例（注入验证时确实漏下过 17 个 `pgrole*` 角色）。

### 三处修法

1. **测试自己造并列**：建 5 个角色后
   `UPDATE roles SET created_at='2020-01-01' WHERE id = ANY($1)`。
   取 5 个而不是 3 个 —— `created_at=2020` 让它们排在全表最前，
   页长 2 时并列组横跨**两个**页边界，组内次序一抖就必然重复或缺失。
2. **总数与 total 独立对账**：`paged_small.len() == total`。
   否则两种页长可能**同样**漏掉同一批，集合比对一起假绿。
3. **先清理再断言**：采集完立刻 `delete_role()`，然后才做断言。
   断言恰恰会在"分页真的有 bug"时失败，清理写在断言之后等于永远不执行。

### 注入验证

去掉 `, r.id ASC` 后：翻页只取回 **163/218** 条、大量重复 id，测试红；
且这次失败后 `pgrole*` 残留为 **0**。恢复 `id ASC` 后绿、残留 0。

> 教训：凡是"排序/分页/去重"类守卫，必须由测试自己造出触发条件，
> 否则它测的是当前数据的巧合。角色累积到 220+ 反而暴露了这个假阳性——
> 数据脏到一定程度，好测试会自己现形。

---

## v0.18.0 门禁结果（全部绿）与两处收尾修正

### 门禁

| 项 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --all-targets -- -D warnings` | ✅ 零警告 |
| `cargo test --lib` | ✅ 77 |
| 集成（非 ignored） | ✅ 12 |
| 集成（`--ignored --test-threads=1`） | ✅ **129**（含上一轮那两条红） |
| 前端 lint（eslint） | ✅ 0 error（`env.d.ts` 1 个既有 warning） |
| 前端 typecheck | ✅ |
| 前端 vitest | ✅ 155（18 个文件） |
| 前端 build | ✅ |
| e2e | ✅ **11/11 套件**，其中 v018 套件 19 条 |

### 收尾修正一：不要把 prettier 的重排混进提交

改动复核时发现 `register/index.vue` 与 `system/user/index.vue` 的 diff 有 **800+ 行**，
而语义改动只有几十行——`<script>`/`<style>` 整体被缩进，文件头注释被压成
`/** * 用户管理页面 * * 仅 admin ... */` 一行，`<template #icon>` 被拆成
`><n-icon>...</n-icon
></template>`。

查证：仓库有 `frontend/.prettierrc`，但 `npm run format` **不是门禁**（门禁是
`npm run lint`，即 eslint，它一直通过）；而 `npx prettier --check "src/**/*.vue"`
有 **30 个文件**不合规——说明 prettier 从来没在这个仓库真正落地过，
`.prettierrc` 是摆设。

处理：`git checkout` 恢复这两个文件，按仓库既有风格（`<script>` 内容顶格、
import 不排序）重新手工施加语义改动。diff 从 799 行降到 **90 行**，
`git diff | grep -cE "^\+  (import|const|function)"` = 0 确认无整体缩进。

> 教训：改 `.vue` 时不要顺手 `npm run format`。本仓库的格式化约定由
> eslint 管，prettier 一跑就是全文件重排 + 注释压行。

### 收尾修正二：`///` 会被导出成对外文档

`src/model/user.rs` 里原本把开发笔记写进了 `///`，而 `RegisterRequest` 正是
`utoipa` 导出的 OpenAPI schema。实测 `GET /api/openapi.json` 的
`password` 描述变成了三段：

```
密码（8-128 个字符，且至少含大写/小写/数字/符号中的两类）

v0.11.0 把策略从"至少 6 个字符"收紧后，**这条注释没跟着改**，…
```

改法：`///` 只留调用方需要的规则，开发过程的话改成 `//`（不进文档），
并在该处留注释提醒"`///` 会被 utoipa 导出成对外描述"。
复核后三条描述都干净了。

### 一处自伤：e2e 跑到一半时我改了前端源码

v018 套件首次跑出 2 条 FAIL，placeholder 是旧的 `"3-50 个字符，字母或数字"`。
原因不是代码错，是**时序**：我在 e2e 跑到 v017 时执行了
`git checkout` 恢复那两个 `.vue`，vite HMR 把旧版 serve 给了浏览器。

`git checkout` 之后确认 vite 已切到新代码（curl 模块源码只剩
`USERNAME_PLACEHOLDER`），重跑该套件 19/19 全绿。

> 教训：**e2e 运行期间不要动前端源码**。要改就等整轮结束——
> 那 2 条 FAIL 是我自己制造的，不是产品缺陷。

---

## v0.18.0 收尾：e2e 连着跑必然红，根因是我自己的启动参数

### 症状

单独跑每个套件都绿，整轮 `node e2e/run.mjs` 却是 8/11，失败的固定是
`v010` / `v011` / `v016`。连着复现三次，症状完全一致。

### 真实原因：IP 限流打满

抓完整日志（`node e2e/run.mjs > /tmp/e2e_full.log 2>&1`）才看到真凶：

```
Error: v0958xt9l_operator 登录失败: 429 "请求过于频繁，IP 限流: 101/100 (窗口: 60s)"
FAIL  吊销会话成功（真实走 /auth/logout）  ::  status=429
```

一套 e2e 从同一 IP 打后端的请求量远超 `RATE_LIMIT_IP_MAX` 默认的 100 次/分。
429 会顺着套件的断言一路放大——"无控制台错误""无意外 4xx/5xx""令牌已被清除"
全变红，看起来像七八个功能坏了，实际只有一个限流阈值。

**`e2e/README.md` 早就写明必须 `RATE_LIMIT_IP_MAX=100000` / `RATE_LIMIT_USER_MAX=100000`**，
还专门警告了"超了会表现为无控制台错误这两条假红"。是我启动后端时漏了这两个变量。

按文档重启后端，整轮 11/11 全绿，exit=0。

> 教训：启动后端去跑 e2e 时，照抄 `e2e/README.md` 的前置段，
> 不要凭记忆敲环境变量。限流类阈值一旦不对，**失败会以"功能坏了"的形式出现**，
> 比"直接启动失败"危险得多。

### 顺带修掉：v016 中途失败后永久不可重跑

排查过程中撞上一个独立的真缺陷。`v016-dict-controls-work.mjs` 的
`CODE = 'custom_type'` 是**固定值**（demo 页的 `DictSelect` 读它，换掉就没意义了），
而清理只写在套件**末尾**：任何断言 panic，`seed()` 撞上唯一约束
`dict_types_code_key` 直接 500，于是这个套件**从此再也不能跑**——
一次中途失败被放大成永久性假红。

改法：抽出幂等的 `purgeFixture()`（按 code 找并删，删类型会级联删字典项），
**开头也调一次**。

注入验证：手工造一个带 3 个字典项的残留 `custom_type`，
去掉开头的 purge 后跑 → `建字典类型失败 500` 红；
恢复 purge → `清掉了上一轮残留的 1 个 custom_type 夹具` + 24/24 绿、残留 0。

11 个套件里只有 v016 用固定唯一标识符，其余都用随机后缀，这类问题仅此一处。

---

## v0.19.0 缺口分析（2026-10-03，只读实测，未动代码）

v0.18.0 已本地提交（`81055867`，未推送）。下面结论全部来自**真实 HTTP 调用**，
不是读代码猜的。

### 主题候选：没人点过的按钮，永远不知道它是坏的

**实测发现：`GET /api/admin/export/users` 每个调用都返回 500。**

```
$ curl -s -w '%{http_code}' /api/admin/export/users -H "Authorization: Bearer $TOK"
500  {"code":500,"data":null,"message":"服务器内部错误"}

后端日志: 内部错误: 查询用户失败: no column found for name: must_change_password
```

- **根因**：v0.11.0（`7d6898ff`）给 `User` 结构体加了 `must_change_password`，
  而 `src/controller/demo.rs:51` 的导出用的是**显式列名的裸 SQL**：

  ```rust
  "SELECT id, username, email, password_hash, is_active, created_at, updated_at FROM users"
  ```

  没跟上。**整整七版，每个调用都是 500。**
- **用户可见**：`frontend/src/views/demo/backend.vue:60` 有"导出"按钮调它，
  失败被 `catch` 吞成 `message.error('导出失败')`——管理员只知道导出坏了，
  不知道坏在哪，也看不出已经坏了七版
- **零测试覆盖**：`grep -rn "export/users" tests/ e2e/` 无结果

### 为什么能烂七版：承重测试只覆盖写端点

`tests/api_integration.rs:7554` `every_documented_write_operation_is_covered_by_the_audit_test`
用 `COVERED` 清单锁住了**每一个文档化的写端点**（23 条），漏一个就红。
但它是 **write operation**——**读端点没有任何等价守卫**。

`export/users` 是 `GET`，于是它天然落在守卫之外，烂了也不会有人知道。

### 实测：逐个调用未覆盖端点，只有它坏

对 OpenAPI 里 39 个端点做了一遍集成测试/探针覆盖扫描，再逐个真实调用：

| 端点 | 状态 |
|---|---|
| `GET /api/admin/export/users` | **500** |
| `GET /api/admin/monitor/system/export` | 200（真 xlsx） |
| `GET /api/admin/monitor/alerts` | 200 |
| `GET /api/admin/dict/cached` | 200 |
| `GET /api/dict/{code}/items` | 200 |
| `GET /api/admin/test` | 200（v0.5 RBAC 演示残留，暂不动） |
| `PUT /users/{id}/status` | 200 |
| `GET /users/{user_id}/roles` | 200 |

（`POST /users/{id}/roles` 与 `/reset-password` 的 400 是**我猜错 DTO 字段名**
——后端要 `role_name` / `password`，报的是 `missing field` 并指名，
这符合 v0.12.0 那套统一错误格式，**不是缺陷**。）

### 建议的 v0.19.0 范围

1. **修 `export/users`**：SELECT 补 `must_change_password`。
   更进一步：该用 `SELECT *` 或共享常量列表，避免下次加字段再烂一次
2. **把守卫从"写端点"扩到"全部端点"**：新增一条承重测试，
   要求 OpenAPI 里每个端点都被至少一条用例真实调用过。
   这条测试本身要先能抓住当前的 `export/users`
3. 顺带清掉 `operator_with_codes` 的 20 处调用点不清理（累计 270 角色/424 账号），
   否则读端点的 `total` 类断言会长期被脏数据打红

### 注意

- 测试库 `axum_api_test` 已累积 **270 角色 / 424 用户 / 80 字典类型 / 12175 审计**。
  上面的实测都已清理自己的探针数据，但历史累积没清

---

## v0.19.0 缺口扫描（2026-10-03，只读实测，未动代码）

已推送 `1a2772de..00ed3ef2`。下面结论全部来自**真实 HTTP 调用**，不是读代码猜的。

### 缺口一：`GET /api/admin/export/users` 自 v0.11.0 起每个调用都 500

（承接上一节，此处只补一句实测确认）

```
内部错误: 查询用户失败: no column found for name: must_change_password
```

根因是 `src/controller/demo.rs:51` 的裸 SQL 显式列名，没跟上 v0.11.0 的字段新增。
零测试覆盖，承重守卫 `every_documented_write_operation_is_covered_by_the_audit_test`
只管**写**端点，而它是 GET——所以烂七版没人知道。

### 缺口二（新发现）：用户名/邮箱不做大小写归一，而角色名做

**同一次运行里的决定性对比：**

```
POST /api/auth/register  {"username":"CaseProbe",...}  → 200，username="CaseProbe"
POST /api/auth/register  {"username":"caseprobe",...}  → 200，username="caseprobe"   ★ 两个独立账号

POST /api/admin/roles    {"name":"CaseRole",...}       → 200，name="caserole"（已被归一）
POST /api/admin/roles    {"name":"caserole",...}       → 409 角色名「caserole」已被占用
```

- 角色侧有 `normalize_role_name()`（`src/model/role.rs:41`）：trim + `to_lowercase`
  + 长度 + 控制字符，写入前统一。
- 用户侧**没有任何归一**：`find_by_username` / `find_by_email` 都是裸 `WHERE username = $1`，
  写入也直接 bind 原值。`users_username_key UNIQUE (username)` 在 Postgres 里
  **大小写敏感**，所以 `Admin` 与 `admin` 合法共存。
- 邮箱同理：`CaseTest@Example.com` 与 `casetest@example.com` 同时创建成功，
  且**分别登录到两个不同的 id**。

**实质影响（实测）**：自助注册路径可以造出

```
注册 Admin  → 200      注册 ADMIN → 200      注册 aDmIn → 200
库里：ADMIN / Admin / aDmIn / admin  四个账号并存
```

用户管理页里这四个**肉眼无法区分**。管理员在列表上看到 `Admin`，
无法判断它是不是真 admin；钓鱼、社工、"给 admin 绑个角色"这类操作都可能被引到伪造账号上。
这不是理论风险，是一次注册请求的事。

### 已实测确认**不是**缺口的部分（避免下版重复排查）

- **分页校验扎实**：`page_size` 超 200 → 400「每页条数必须在 1-200 之间」；
  `page=0/-1` → 400「页码必须大于 0」；`page=99999` 返回空 items 且 total 正确
- **SQL 注入不成立**：`sort_by=id; DROP TABLE users;--` 不报错，
  因为 `PaginationParams::get_order_sql()` 有 `allowed_fields` 白名单，
  非法字段回落 `created_at`，方向也只认 `asc/ASC`
- **级联删除到位**：删字典类型会清掉其下 dict_items；删角色会清空 role_menus
- **空白用户名被挡**：`"  spaced  "` → 400「用户名只能包含字母、数字、下划线和连字符」
  （这是 v0.18.0 共享字符集规则的成果）
- **并发写**：两个 PUT 同改一个角色都返回 200，后写覆盖。无乐观锁，
  但"后写胜出"是常见可接受设计，本版不动

### 环境注意

轮次之间后端与 vite 进程会被重置。重启后端必须带
`RATE_LIMIT_IP_MAX=100000` / `RATE_LIMIT_USER_MAX=100000`（见 `e2e/README.md`），
否则整轮 e2e 会因限流假红。

### 建议的 v0.19.0 范围

1. 修 `export/users` 的 SELECT（并考虑 `SELECT *` 或共享列常量，避免下次加字段再烂）
2. 给用户名/邮箱加归一（trim + 小写），照抄 `normalize_role_name` 的形状；
   同时把唯一性检查改成归一后比较。**须处理存量脏数据**：
   库里可能已有仅大小写不同的账号，迁移要能报出冲突而不是静默合并
3. 把承重守卫从「写端点」扩到「全部端点」，先让它抓住当前的 `export/users`
4. 清理 `operator_with_codes` 的 20 处泄漏（测试库已累积 270 角色/424 用户）

**注意顺序**：第 2 项涉及迁移与存量数据，风险最高；1 与 3 是低风险独立项。
若要拆，建议先做 1+3 拿一个干净版本，2 单独一版。

## v0.19.0 四项全部完成（用户指令「合在一起做 v0.19.0」）

起始 git 状态：HEAD `9f5e05d5`，分支 `master`。
纪律：提交后**不推送**，等用户指令统一推。

| # | 项 | 状态 |
|---|---|---|
| 1 | 修 `export/users` 的 SELECT + 根治列名漂移 | ✅ `0c26ad6c` |
| 2 | 承重守卫从写端点扩到全部 50 个端点 | ✅ `0c26ad6c` |
| 3 | `operator_with_codes` 的 19 处泄漏 | ✅ `3c26640b` |
| 4 | 用户名/邮箱大小写归一 + 迁移 013 | ✅ 本次提交 |

**尚未推送**：`0c26ad6c`、`3c26640b`、第 4 项的提交。
**尚未发布**：v0.19.0 的 tag / Release / CHANGELOG / 版本号都没动——
等用户指令再推。推送前记得版本号 `Cargo.toml` + `package.json` + CHANGELOG 一起改。

### 已完成：第 1 项 — `export/users` 的 SELECT + 根治列名漂移

`repository::user::USER_COLUMNS` 常量（照抄 `repository::menu::MENU_COLUMNS` 的形状），
原先 10 处手写列名全部改用它：user.rs 8 处 + `controller/demo.rs` 1 处 + `service/rbac.rs` 1 处。
`grep -rn "id, username, email, password_hash, is_active, must_change_password, created_at, updated_at" src/`
现在只剩常量定义那一行。

实测（真实 HTTP）：
- 修复前旧二进制：`GET /api/admin/export/users` → **HTTP 500**
  `{"code":500,...,"message":"服务器内部错误"}`（底层 `no column found for name: must_change_password`）
- 修复后：→ **HTTP 200**，`23555` 字节，`content-type: application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`，
  magic `504b`，解出 10 个 zip entry、含 `sheet1`（真 xlsx，不是错误信封）

### 已完成：第 2 项 — 承重守卫从「写端点」扩到「全部端点」

新测试 `every_documented_endpoint_is_reachable_without_a_server_error`
（`tests/api_integration.rs`，`#[ignore]`），从 `openapi_operations()` 派生**全部 50 个**端点
（此前写端点守卫只派生 `POST|PUT|DELETE`；文档里 GET 有 22 个，一个都不在它视野里）。

判据分层：
- **GET（读端点）必须 2xx** —— 无必填请求体，带合法令牌就该跑通；4xx 同样是缺陷
- **非 GET 只要不是 5xx** —— 发 `{}` 让它停在校验层（400），既走到处理函数入口又不改真实数据
  （实测：20 个带体端点全 400；`DELETE /api/admin/{users,roles,menus,dict/types,dict/items}/{NIL}` 全 404，不误删）
- 路径参数由文档派生：uuid 填 `VALID_UUID`（不存在的行），非 uuid 的 `{code}` 填 `NON_UUID_PARAM_PROBE`
- 必填 query 参数也补（当前只有 `dict_type_id`），否则查询提取器先回 400，探针走不到处理函数
- 自查断言（探针表可能整体失效）：`>=50` 总数、`>=22` 读、`>=28` 写

两个有副作用的端点已处理：
- `POST /api/auth/logout` 会让令牌失效 → 探针跑完**重新登录**换新令牌
- `POST /api/admin/monitor/metrics/reset` 清空指标 → 无影响，每个读指标的用例自己先重置

**两个踩过的坑（都靠编译器/自查抓住，不是猜的）**：
1. 我最初把 50 个 `Request` 在循环前一次性构造好，每个都持有同一个旧令牌。
   logout 一失效，后面按 `(method, path)` 排序的 `POST /api/auth/register` 与若干 PUT 全 401 假红。
   编译器报 `value assigned to token is never read` 暴露了它——已改成**发送时才构造**请求。
2. 自查阈值我先写成 `write_count >= 30`，实际 28，测试当场红。已改为 22/28。

缺陷注入验证（已做）：
把 `demo.rs` 的 SQL 改回漏掉 `must_change_password` 的手写版 →
```
GET /api/admin/export/users 返回了 500 Internal Server Error：{"code":500,...}
  · 处理函数内部出错，这正是 export/users 烂了七版的形态
```
测试红，报错直接点名端点。回滚后复跑绿。

### 已完成：第 3 项 — `operator_with_codes` 的 19 处泄漏

实测：库里累积 **212 个操作员角色 / 424 个账号**（整库 270 角色 / 424 用户，即绝大多数都是它），
而 142 个用例**全绿**。测试不检查自己留下的垃圾，就永远发现不了自己在漏。

新增 `cleanup_operator(app, admin_tok, user_id, role_id)`，19 个调用点全部配上。
**走 API 而非 SQL，且顺序是先删用户后删角色**——实测反序会被挡回 400
「仍有 1 个用户使用该角色，请先调整这些用户的角色」。那条拒绝是**有意设计**：
`user_roles` 的 `ON DELETE CASCADE` 会**静默**剥掉这些用户的角色，
让人变成"没有任何角色"的用户而不自知。清理要顺着它的意思，而不是绕开它。

**为什么走 API 而不像 `cleanup_holder` 那样走 SQL**：两者授权状态不同。
本夹具角色挂的是 `system:user:list` 这类**真实**权限码，admin 按种子持有全部，
授权下界（能授予的 ⊆ 已持有的）自然放行；`cleanup_holder` 的角色挂
`tmp:*:priv:*` 一次性专属码，admin 按设计不持有，才只能走 SQL。
两种形态都实测删得掉。走 API 的额外好处：不绕过被测逻辑。

守卫 `the_permission_code_fixtures_leave_no_holder_behind` 的范围从一支扩到两支。
此前它的注释写着"`operator_with_codes` 那是另一笔账、另一个版本的活"——就是本条。

**关键前置：给夹具加了 `OPERATOR_FIXTURE_PREFIX = "opf_"`。** 不加就没法精确圈定：
原命名 `{prefix}_role_{hex}` 与 `grantee_role_*`、`strong_role_*`、`tmp_holder_role_*`
撞形状，守卫要么长期误报、要么被人加豁免——两者都等于没有守卫。
**顺序不能反**：先有前缀与清理，最后才放大守卫范围，否则本条会长期红。

`the_retention_endpoint_obeys_the_log_permission` 原本已有手工清理
（删用户 + 删角色），改成调 `cleanup_operator`，否则会重复删导致 404。

缺陷注入验证（已做）：抽掉 19 处中的 1 处清理 →
- 对应测试**仍然通过**（这正是问题：泄漏是静默的）
- 守卫当场红并点名残留：`opf_hr_role_c634c48b` / `opf_hr_user_d7b5b3b4`
回滚、清掉注入的残留后复跑绿。

19 个测试逐个跑过全绿；跑完后 `opf_%` 残留为 0。
顺带清掉历史累积 181 角色 + 181 账号（只按 19 支夹具前缀精确匹配，
种子 `admin`/`user` 与其他夹具一律不碰）。库里角色 270 → 87。

**踩坑**：我用脚本按"列 0 的 `}` 即函数结尾"定位插入点，先用括号配平验证了 19 处全部闭合，
但真正写入时用了**替换**而不是**插入**，把 18 个函数的收尾 `}` 覆盖掉了。
教训：这类批量改写，定位方法要验证，**写入方式更要单独验证**——
改完必须 `cargo fmt` + `cargo check` 过一遍，不能只看脚本输出。

### 第 4 项：用户名/邮箱大小写归一 + 迁移 ✅ 已完成

**为什么这是本版风险最高的一项**：用户名不只是展示用，它是**登录键**，
也是管理员在用户列表里辨认账号的依据。而 Postgres 的 `UNIQUE(username)`
是**大小写敏感**的——不归一的话 `Admin` / `ADMIN` / `aDmIn` 能与真 `admin`
并存，而**自助注册一次就能造出来**。管理员在列表上看到 `Admin` 无从判断它
是不是真 admin，于是"给 admin 绑个角色"、"重置 admin 口令"这类操作会被引到
伪造账号上。这不是理论风险，是一次注册请求的事。

#### 归一函数（`src/utils/validation.rs`）

- `normalize_username` / `normalize_email`：trim + 小写，**先归一再校验**，
  保证"被校验的就是被存下的"。
- `normalize_login_input`：trim + 小写、**不校验**。登录框接受用户名或邮箱两者，
  用户名规则会把合法邮箱判非法；且不区分"格式错/不存在"以免给爆破者枚举信号。
  形状照 `src/model/role.rs:41` `normalize_role_name`。

**三条写入路径全部改走归一值**：`service/auth.rs::register`、
`controller/user.rs::create`、`controller/user.rs::update`。
查重因此自动变成"归一后比较"，不需要额外再写一条大小写不敏感的查重——
两处规则迟早会走偏。审计记归一后的值：有人拿 `Admin` 撞已存在的 `admin`
会留下"针对真账号的 409"，正是调查想要的信号。

**读取路径**：`service/auth.rs::login` 开头算一次 `login_input`，**全流程复用**
（查库 + 限流 key + 3 处审计）。分头各算迟早对不上账。
注意 `account_scope` 本来就已经 `.to_lowercase()`，即登录失败计数的 Redis key
**早就**大小写不敏感了——本项补的是"账号本身"那一层，不是限流那一层。

#### 迁移 `013_normalize_user_identities.sql` — 冲突时报错，而不是跳过

**不能照抄 008（角色名）的做法。** 008 对冲突行是**跳过**的，理由写在它的注释里：
「迁移期不该把整个应用卡在起不来」。角色名这样可以，用户名不行——
跳过后那行会变成**登录不到的孤儿账号**：归一后所有写入都是小写，
`find_by_username("admin")` 只命中小写那行，旧 `Admin` 行再也登不进去，
却仍挂在库里、仍持原角色，且在用户列表里与真 admin **肉眼无法区分**。
那等于"声称修好了大小写唯一性"，实际反而留下一个更难查的冒充入口——比迁移前更糟。
合并账号会丢权限（`user_roles` 按 user_id 关联），必须管理员显式决定。
所以选 `RAISE EXCEPTION` + 把冲突行列出来：应用起不来是可见的故障，
一个静默的冒充入口不是。

迁移四段：①归一无冲突的 username ②归一 email ③DO 块冲突则报错并列出行
④建 `lower()` 函数唯一索引做第二道防线。
裁空白用 `regexp_replace(x,'^[[:space:]]+|[[:space:]]+$','','g')`，
**不写 `btrim(name,'[:space:]')`**——那是字符集合，会把 admin 裁成 dmin（见 008 注释）。

#### 迁移三个场景都单独验过（均已回滚）

- A 仅归一无冲突 → 成功，`'  MixedCase  '` → `mixedcase`
- B 仅用户名冲突 → 报 `用户名冲突（归一后）: caseprobe → CaseProbe, caseprobe`
- C 仅邮箱冲突 → 报 `casetest@example.com → CaseTest@Example.com, casetest@example.com`

#### 🐛 写测试时发现并修掉的真缺陷：约束名只认旧的

`repository/user.rs` 按**约束名**把唯一冲突翻成 409，只认迁移 001 的
`users_username_key` / `users_email_key`。而 013 新建的函数索引叫
`users_username_lower_key`，**仅大小写不同的插入撞的是它**——实测：

```
INSERT ... VALUES ('ADMIN', ...) →
ERROR: duplicate key value violates unique constraint "users_username_lower_key"
```

只认旧名字的话这条会落到 **500 而不是 409**：013 注释里承诺的"第二道防线"
拦住了却报 500，等于把一个可诊断的冲突变成"用户说系统坏了"。
已抽 `conflict_from(e, action, username_msg, email_msg)` 同时认四个名字。
两处调用点的措辞不同（`create` 说"已被注册"、`update` 说"已被占用"）是**既有**对外文案，
由调用方传入，没有顺手统一——改它对本次缺陷没帮助，却会让已有的文案比对失效。

这条路径走 HTTP 时**永远测不到**（应用侧查重会先一步挡住），
所以测试里直接调 `UserRepository::create` 断言拿到 `AppError::Conflict`——
目的就是不让这条路径保持不可见。

#### 测试

`tests/api_integration.rs` 新增 3 条 + `validation.rs` 的 `mod tests` 新增 4 条：

| 测试 | 覆盖 |
|---|---|
| `user_identities_are_normalized_on_every_write_path` | 注册/建号/改号**三个入口**各验归一；大小写变体 409；`ADMIN`/`Admin`/`aDmIn` 冒充被挡；落库确为小写；冲突时不得留下半改的行 |
| `login_accepts_any_casing_of_username_and_email` | 用户名与邮箱**两条查询路径**都大小写不敏感（登录框两者都接受） |
| `the_database_rejects_identities_differing_only_in_case` | 函数索引在 DB 层拒绝；钉住撞的是 `*_lower_key`；仓储层翻成 409 |
| `normalization_happens_before_validation` | 顺序：49 个 a + `İ` 归一后 51 字符必须拒（否则放行一个存下去就超长的用户名） |
| `username_is_trimmed_and_lowercased` / `email_is_trimmed_and_lowercased` / `login_input_is_normalized_but_never_validated` | 归一规则本身；登录输入**不校验**（`ab`、带空格的非邮箱都放行给查询） |

写测试时又踩了一次同样的坑：`İ`(U+0130) 小写后是 `i` + U+0307 组合上点，
而 **U+0307 是 Mn（组合记号），不是字母数字**，所以 `is_alphanumeric()` 对它返回 false。
我原先把它当成"归一后更宽松"的例子写进断言，测试当场红——真实结论是归一对它只会**更严**。
已改成用两端空白论证"顺序错了会误拒"，并把这条事实写进注释。

#### 前端同步（原本会漂移）

`frontend/src/utils/accountRules.ts` 的 `usernameIssues` / `emailIssues`
原本校验**输入框原文**，而后端现在校验**归一后的值**——不一起改的话，
前端会红着拦下一个后端明明接受的输入（`  alice  ` 的空格既不在字符集里、
长度也超了）。已加 `normalizeUsername` / `normalizeEmail` 并让两个
`xxxIssues` 先归一再判。

连带改了 Rust↔前端契约测试 `username_policy_agrees_with_the_frontend_rules`：
它原来调 `validate_username`，现在调 **`normalize_username`**——
对账的必须是前端实际走的那条路径，否则报错只会说"不一致"却指不出哪边错了。

契约样例表加了 `{ name: '  alice  ', ok: true }` / `{ name: 'ALICE', ok: true }`
/ `{ name: '   ', ok: false }` 三条，钉住"校验的是归一后的值"。

**又踩了一次"字面量 vs 转义"的坑**：我本来想加 `{ name: '\tadmin\r', ok: true }`，
但契约测试是按**文本**解析这张表的（找下一个单引号截断），它会拿到字面的
反斜杠 t 而不是制表符——JS 侧 trim 掉的是空白、Rust 侧看到的是 `	admin
`
这串反斜杠，两侧结论必然相反，而报错只会说"不一致"。已删掉该条并在表旁
写明原因（与表里既有的"必须写字面量不能写 `.repeat()`"是同一类坑）。

前端另加 5 组 vitest 断言（`normalizeUsername`/`normalizeEmail` +
两端空白与大小写不影响校验结论 + 长度按归一后字符数计）。

#### 缺陷注入验证（每处都做了，会红并回滚）

| 注入 | 结果 |
|---|---|
| 三个归一函数全退回 `raw.to_string()` | 2 条集成测试红（红在 trim 那一关） |
| **只去掉 `.to_lowercase()`，保留 trim** | 2 条红，且**红在大小写断言上**（隔离出"大小写"这一个属性） |
| 约束名只认 `*_key`（退回缺陷版） | `the_database_rejects_identities_differing_only_in_case` 红：`InternalServerError(... "users_username_lower_key")` |
| 契约样例表把 `  alice  ` 改成 `ok: false` | 契约测试红：`"  alice  "：前端期望 false，后端实际 true` |

第一次注入只去掉小写时测试红在**别的地方**（trim 那一关），
说明"大小写"这个属性其实没被单独覆盖，于是做了第二次更精确的注入。

#### 门禁结果（全绿）

| 项 | 结果 |
|---|---|
| `cargo fmt --check` | ✅ |
| `cargo clippy --all-targets -- -D warnings` | ✅ 0 warning |
| `cargo test --all-targets` | ✅ 81 单元 + 12 非 ignored |
| `cargo test --test api_integration -- --ignored --test-threads=1` | ✅ **133 passed / 0 failed** |
| `pnpm lint` | ✅ 0 errors（`env.d.ts` 1 个历史 warning） |
| `pnpm typecheck` / `pnpm test` / `pnpm build` | ✅ / ✅ **161 passed** / ✅ |

**真实 HTTP 复核**（新二进制 session 46736，18080 端口，真实 PG+Redis）：
注册 `  LiVeProbe  ` → 200 存成 `liveprobe` / `liveprobe@example.com`；
`LIVEPROBE`/`LiVeProbe`/`liveprobe` 注册 → 全 409；
`ADMIN`/`Admin`/`aDmIn` 注册 → 全 409；
登录 `liveprobe`/`LIVEPROBE`/`LiVeProbe`/`  LiVeProbe  `/`liveprobe@EXAMPLE.COM`
→ 全 200；错口令仍 401；管理员建号 `  AdMinProbe  ` → 存 `adminprobe`；
**`export/users` 仍 200 / 17288 字节 / magic `PK\x03\x04`（第 1 项未回退）**。

探针数据已按精确用户名删净；`username <> lower(username) OR email <> lower(email)`
的存量计数为 **0**。

> 缺陷注入跑红的 3 次会在清理前中止，库里留了 3 个大写账号
> （`MiXeDcaselogin_*` / `MiXeDnorm_*` / `dbcasedbcase_*`，各带 1 个 `user` 角色）。
> 已确认是本轮注入产生并删净。**注入型红测必然留残留**，收尾记得查一次。

### 已确认**不是**缺口（别重复排查）

分页校验扎实（`page_size>200`/`page<=0` 都指名 400）；SQL 注入不成立
（`get_order_sql()` 有 `allowed_fields` 白名单）；级联删除到位；空白用户名被字符集规则挡下；
并发写后写胜出无乐观锁（可接受，不动）。

### 环境注意

轮次之间后端与 vite 进程会被重置。后端 session 用 exec 长驻方式启动（`nohup` 会被回收）。
必须带 `RATE_LIMIT_IP_MAX=100000` / `RATE_LIMIT_USER_MAX=100000`，否则整轮 e2e 因限流假红。
JWT 密钥在 `/tmp/axum_jwt_secret.txt`。文档端点总数是 **50**，不是早前 handoff 里写的 39。
