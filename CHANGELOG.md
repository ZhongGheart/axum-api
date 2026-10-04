# Changelog

本项目遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [0.22.0] - 2026-10-04

主题：**把「系统行为」从环境变量和代码常量里搬出来，让管理员在界面上改。**

迁移 `016` 新增 `system_settings` 参数表，`users` 表新增 `password_changed_at` 列。
七个参数覆盖口令复杂度、口令有效期与登录失败锁定策略。

### 范围取舍：这一版只做了 C2 + C3

ROADMAP 的 v0.22.0 原列了四项（部门树 / 参数表 / 口令策略 / 2FA），
但**它自己的排序理由否掉了其中两项**：

- 「C1 独立但量大，可以和 C2 并行，但**不要同版**——树形递归删除的边界情况
  （父子循环、跨部门角色授权）需要独立的测试预算」
- 「C4 2FA ... 这是独立且用户感知强的大功能，**建议单独占一版**而不是塞进 v0.22.0」

所以本版 = **C2（系统参数配置表）+ C3（口令复杂度与过期策略）**。C3 硬依赖 C2
（策略要可配才有意义），两者同版自洽。**C1 顺延到 v0.23.0。**
这是按路线图最经得起推敲的那句执行，不是自行缩小范围。

### 参数由代码定义，不由管理员凭空造

与 `model/permission.rs` 同构：`SETTING_DEFS` 是**单一数据源**，
DB 只负责持久化取值。管理员能改的是「已定义参数的取值」，
不能新造一个参数——否则就多了一套没有校验、没有边界的自由格式键值对。

每个参数**必须写明 `consumed_by`**（被哪段代码读），并在参数页显示出来，
由单测 `every_setting_documents_its_consumer` 钉住。理由是 v0.16.0 的整版教训：
**一个没有效果的开关比没有开关更糟**——管理员会以为策略已生效而据此放宽其他控制。

### 部署配置 vs 管理员覆盖：优先级由数据本身承载

两个来源都会「想」决定 `login.max_failures`，顺序写错则两种结局都很糟：

- **参数表无条件优先** → 部署时用环境变量设的值被**静默忽略**。
  实测（本版首次集成时真的踩到）：测试与部分部署靠 `LOGIN_MAX_FAILURES=3`
  构造低阈值，种子值 10 直接盖掉它，于是「失败 3 次应被锁定」变成 200，
  **而日志里一个字都没有**——爆破防护看起来在工作，实际从未生效。
- **环境变量无条件优先** → 管理员在界面上改了参数，重启后又变回去，
  「写入成功」却永远不生效，正是 v0.16.0 关掉的那类缺陷。

最终顺序：**管理员显式改过 → 参数表的值；否则 → 部署配置的值**。
判定靠 `updated_by IS NOT NULL`（种子写入行为 NULL），
所以这个区分由**数据本身**承载，不依赖任何内存状态。
参数页用 `source` 字段（`admin`/`env`/`default`）如实告诉管理员「这个数字是谁定的」。

### 两条不可让步的约束

**1. 口令策略绝不进入登录校验路径。** 只在设置/修改口令时校验。
抬高门槛的当天，存量弱口令用户**不能被锁在门外**。
后端约束测试 `password_policy_is_not_applied_to_login_verification` 钉住。

**2. 口令过期不阻断登录。** 复用 v0.11.0 的受限令牌机制
（`must_change_password=true`）：登录仍成功，受限令牌只放行 password/logout/me。
理由：没有邮件通道，拒绝登录等于账号永久锁死，而用户连「为什么被拒」都看不到。

### 公开端点刻意只暴露 4 条规则

`GET /api/settings/password-policy` 是**公开端点**（注册页处于未登录状态）。
但只返回 `min_length` / `max_length` / `min_char_classes` / `require_mixed_case`，
**不含** `expiry_days` 与锁定阈值——把「账号多久被锁一次」暴露给未登录端点
等于给爆破者一个可直接调的参数面板。
由 `the_password_policy_endpoint_is_public_but_leaks_no_lockout_thresholds` 钉住。

### 顺手修掉的两个会让新功能「静默失效」的缺陷

**存量库拿不到新权限码。** `seed_menus_if_empty` 只在 `menus` 表为空时写整棵树，
而**已存在的部署 `menus` 非空**，于是新增的 `/system/setting` 页面菜单永远种不进去。
后果是连锁的：权限码按 `parent_path` 解析父菜单，解析不到就**只打一行 warn 然后跳过**——
`system:setting:list` / `system:setting:update` 在存量库上根本没种出来，
管理员手动授权也授权不了，直接 403。新增 `backfill_late_added_menus` 补齐。

**测试自身有三处缺陷**（不是产品缺陷，但会让整轮结果不可信，必须记下来）：
`put_setting_raw(...)` 漏 `.await` 导致清理不执行、状态串味；
权限码测试撤销 admin 授权后**未恢复**，污染了后续两条测试；
一条口令样例只写了 7 个字符，触发的是长度检查而非大小写检查。

### 前端：口令策略从常量变成运行时取值

这是本版**最重要的前端决定**。原先 `utils/password.ts` 与 `accountRules.ts`
把 8 位 / 两类字符写死，而管理员现在能在参数页改它们。
不改的后果是**界面提示的规则与后端实际执行的规则分叉**：
用户按提示填一个合规口令、提交后被拒，而报错在说另一件事——界面在主动误导人。

- `passwordIssues(password, policy)` 吃传入策略；省略时用 `DEFAULT_PASSWORD_POLICY`
  （与后端 `PasswordPolicy::default()` 逐字一致）
- `passwordPolicyRules` / `PASSWORD_PLACEHOLDER` / `PASSWORD_MAX_LEN`
  **从常量变成函数**，注册页、改密页、管理员建号三处传入 store 里的策略
- 判定失败原因（`passwordIssues`）与界面要求提示（`describePasswordPolicy`）
  **共用同一个 `classRequirementText`**——两处各写一份时漏改的那处会开始说另一件事
- store 的降级路径：取不到策略时保留回落值且**不抛、不弹错**。
  它是纯提示性增强，为它弹红字会让「服务端暂时不可达」看起来像「注册坏了」
- 参数页**不硬编码参数清单**：能改哪些参数、类型、范围、说明、消费方
  全部由后端下发。写死一份等于把「参数定义」这个单一数据源劈成两半。

**没有引入「口令历史」参数。** 它既非复杂度也非过期，要生效还需新表
+ 改所有写入路径。理由同上：没有效果的开关比没有开关更糟。

### 测试

Rust 单测 108 条；集成测试新增 14 条（含审计覆盖），**全量 176 passed / 0 failed**；
前端用例 180 → **203 passed**（21 个文件）。

**缺陷注入验证**（本仓硬要求）跑了 5 处，逐条确认「注入后必须红」：

| 注入 | 结果 |
|---|---|
| 删掉注册时的 `password_changed_at` 写入 | **首次注入时全绿** —— 暴露了真实测试缺口 |
| 口令过期改成拒绝登录 | ✅ 红 |
| 参数表无条件优先于部署配置 | ✅ 红 |
| 去掉 `backfill_late_added_menus` | ✅ 红 |
| `require_mixed_case` 只改文案不改判定 | ✅ 红 |

第一条值得单独说：注入后 `an_expired_password_yields_a_restricted_token_...` **依然通过**，
因为那条测试自己用 `UPDATE` 把时间戳改成 100 天前——它验的是判定逻辑，
从不验**写入路径有没有写**。而字段为 NULL 时 `is_expired(None)` 按设计判为未过期，
于是这个漏表现为**过期策略对每一个新用户永久静默失效**：
参数页显示着 90 天，界面上没有任何异常，只有安全策略不在了。
补了 `every_path_that_sets_a_new_password_records_when_it_was_set`
（覆盖注册 / 自助改密 / 管理员重置三条路径），重跑注入即红。

### 从 v0.21 升级到 v0.22

见 README 的「从 v0.21 升级到 v0.22」。要点：
迁移 `016` 会新增一张表和一列并回填存量口令时间戳；
`LOGIN_MAX_FAILURES` / `LOGIN_FAILURE_WINDOW` 两个环境变量**从「决定取值」降级为「回落值」**；
默认配置下没有任何用户行为变化。

## [0.21.0] - 2026-10-04

主题：**界面在，但首页从来没出现过。**

用户要求全面梳理 UI、优化交互与配色。梳理过程中发现了两个**代码读不出来、只有真跑起来才暴露**的缺陷，
其中一个让首页从上线起就是一片空白。纯前端改造，**未动后端 Rust**，无数据库迁移、无 API 变化。

### 缺口表

| 缺口 | 实测依据 |
|---|---|
| **登录后首页一片空白** | 布局壳 `Root` 的 `path` 是 `/`，菜单种子数据把首页也注册成绝对子路径 `path='/'`。`buildRoutesFromMenus` 把它当绝对子路径注册，vue-router 里出现一条与父级同路径的子记录 → 父级先命中、子级永不命中 → `router-view` 只渲染出 `<!---->`。**侧栏菜单、面包屑、`/system/user` 等全部正常**，所以一直没人发现；`views/home/index.vue` 那句"后续替换为 Dashboard"的占位卡片**一次都没显示过** |
| 侧栏菜单项显示成函数源码 | `renderMenuLabel` 写成了"返回一个函数"，而 naive-ui 要求它**本身就是**渲染器。naive 把那个函数当成待渲染内容，侧栏显示 `() => appStore.collapsed ? h("span", ...) : option.label` |
| 主色与登录页紫蓝撞色 | `stores/app.ts` 主色 `#2080f0` 是 naive 默认蓝，与登录/注册页的紫蓝渐变并排时像两个产品 |
| 大量组件未纳入主题体系 | `Card / DataTable / Button / Menu / Input / InternalSelection / Layout` 无组件级 theme-overrides，只有全局主色，颜色在不同页面不一致 |
| 暗色下占位文字几乎看不见 | 暗色 `placeholderColor` 为 `#666`，与输入框背景对比度约 **3.4:1** |
| 侧栏六排一模一样的齿轮 | 菜单种子给 6 个菜单都种了 `icon='settings'`，`MENU_ICONS` 命中同一个键 |
| 用户列表头像加载失败时画出碎图 | `n-image` 配空 `fallbackSrc`，加载失败时渲染浏览器碎图图标；28px 的碎图比留白更像故障 |
| 角色页与日志页时间算错一个时区 | 手写 `.replace('T',' ').slice(0,19)`，**带时区偏移时会算错一个时区且看起来完全正常** |
| 面包屑缺层级 | 只显示当前菜单名，多级菜单下无法回溯祖先 |

### 配色与设计令牌

- 主色 `#2080f0` → `#2b5fd9`（偏青靛蓝，避开 naive 默认蓝与登录页紫蓝打架）。
  **改的是显示，不动菜单表的 `icon` 字段**——那是管理员的数据。
- 新增 `PATH_ICONS` **按路径兜底**，解决六排齿轮。
- 补齐 Card / DataTable / Button / Menu / Input / InternalSelection / Layout 的**组件级** theme-overrides。
- `global.css` 补齐表面/文本/状态/边框/圆角语义变量、`focus-visible`、`prefers-reduced-motion`。
- 暗色 `placeholderColor` `#666` → `#848b98`。
- `.user-info:hover` 由写死的 `rgba(0,0,0,.05)` 改走变量（暗色下原本几乎不可见）。
- 监控页与仪表盘的 🟢🔴 状态 emoji 换成 CSS 圆点（emoji 在不同系统上字形与基线都不一致）。

### 首页从占位页变成真仪表盘

4 个指标卡 + 最近操作 + 运行状态 + 刷新按钮。**按 `permissionsStore` 逐项降级**：
没权限的指标**不发请求也不渲染**（不是渲染成 0 或报错），全无权限时给单个空状态。
实测普通用户（`user` 角色）**零 403**。

跳转路径**不写死**：真实路径是 `/system/monitor/system`（不是 `/monitor/system`），
且菜单路径是管理员可改的数据，改为按 component 标识从菜单树反查，查不到就不渲染按钮。

### 其他改动

- 顶栏用上 v0.20.0 的 `avatar_url`（此前硬编码蓝底首字母），头像逻辑抽出 `utils/avatar.ts` 三处共用。
- 新增 `utils/time.ts` 统一时间格式化；用户页原先直出 RFC3339 且在列宽不足时折行。
- 新增 `AuthShell.vue` 供登录/注册共用，抽掉两份重复的紫渐变；标签由左侧固定宽改顶部；
  去掉 `letter-spacing: 4px` 与"登 录/注 册"加空格；标签文案"记住密码"改为**"记住用户名"**
  （实现本来就只记用户名）。
- Logo 由 emoji ⚡ 换成图标；折叠/主题按钮加 tooltip；折叠态菜单项补 `title`。

### 测试

新增 `utils/__tests__/{avatar,time}.spec.ts`，前端用例 161 → **180 passed**。

### 从 v0.20 升级到 v0.21

**无数据库迁移，无 API 变化，无环境变量变化。** 后端二进制不变，升级只需重新构建前端。

唯一的用户可见变化：**首页现在真的会显示内容**。若此前有人报告"登录后首页是空的"，
那是本版修掉的缺陷，不是回归。

### 踩过的坑（留给下一个人）

- `.vue` 导入必须带显式 `.vue` 后缀（`@/components/common/AuthShell` 解析不到）。
- `vue` 文件里有嵌套 `<template>`（slot）时，`s.index('</template>')` 会截到**内层**插槽，
  留下游离的旧模板尾部。Vue 不报错、tsc 也不报错，但页面渲染成空白。**必须用 `rindex`**。

## [0.20.0] - 2026-10-04

主题：**账号自持 + 管理可应急。**

用户对自己的账号**没有任何自助修改能力**，管理员在出事时**没有手动手段**。
两条线都是在已有机制上闭环，不开新战场。详见 [`docs/ROADMAP.md`](docs/ROADMAP.md)。

### 缺口表

| 缺口 | 实测依据 |
|---|---|
| 用户改不了自己的资料 | `/api/auth/*` 下只有 `password` 是 PUT，**没有 profile 端点**；`users` 表 8 列里没有 `display_name` / `avatar_url`；`profile/index.vue` 只有改密三个字段 |
| 管理员找不到"某个角色的禁用账号" | `list_users` 只收 `page/page_size/keyword`，keyword 同时匹配 username+email，只能靠翻页 |
| 文件上传是**悬空 affordance** | 前端 `BaseUpload.vue` 完整可用且已导出，但**无人使用**；后端 `axum` 只开 `features=["macros"]` **没开 multipart**，`tower-http` 也没开 `fs` |
| 账号被锁只能干等 | `clear_login_failures` **唯一调用点在登录成功分支**，管理员**无手动解锁入口**；锁 TTL = `LOGIN_FAILURE_WINDOW` 默认 300s 自过期 |
| "这个人现在在哪些设备上"无从回答 | 登录成功后**不写任何会话记录**，jti 只在登出时进黑名单。JWT Claims 里 `jti` 早就有了，地基是齐的，只差登记 |
| 用户导入只有一个方向 | 只有 `GET /api/admin/export/users`，没有反向。全仓 `import` / `导入` 只命中 TS 动态 import |

### A 线：用户自助

**`PUT /api/auth/profile`** — 迁移 014 加 `display_name VARCHAR(50)` / `avatar_url VARCHAR(512)`，各带 CHECK。

字段级**三态**语义（`Option<Option<String>>`）：不带 = 不改，`null` = 清空，带值 = 设置。
用 `Option<String>` 的话"不带字段"和"display_name: null"都是 `None`——
前端只想改头像时会顺手把展示名也清了，这是一次静默的数据丢失。
仓储用**外层 flag + CASE WHEN** 而不是 `COALESCE`，前端不必先读旧值再原样写回
（回写一个刚被别人改过的旧值就是典型的丢失更新）。

列表筛选新增 `is_active` / `role` 两个维度，`list_filtered` 从 `match keyword` 二分支改为动态拼 WHERE + 顺序绑定。
筛选用 **EXISTS 不用 JOIN**：一个用户可能同时命中多条角色行，JOIN 会让同一用户重复出现并把 total 算大。
两者同时给是 **AND**（"既是 HR 又是禁用的"是一个明确的问法，改成 OR 会返回一批用户没预期的账号）。

**`POST /api/auth/profile/avatar`** — 开 `axum/multipart` + `tower-http/fs`，`nest_service("/uploads", ServeDir)`。

- 文件名由**服务端**生成 UUID，扩展名由 **MIME 白名单**推导。绝不能用原始文件名拼路径。
- 替换头像时删旧文件，而**旧头像必须在写新值之前读出**——写完之后库里已指向新文件，
  回头再读只能读到新路径，"替换掉旧文件"这件事就悄悄失效了。
- 写库失败删掉刚写的文件，否则每次失败都留下一个没人引用的孤儿文件，而用户会以为"至少图还在"。
- `/uploads` 刻意挂在鉴权之外：图片是 `<img src>`，带不了 Authorization 头。
- 启动时 `create_dir_all`，让"部署漏挂卷"在启动时暴露，而不是等第一个用户上传才发现。

### B 线：管理应急

**`POST /api/admin/users/{id}/unlock`** + 新权限码 `system:user:unlock`。
复用登录的同一个归一函数算 scope（`account:{username}` / `account:{email}`），
否则清掉的 key 与写入的 key 对不上。**刻意不碰 IP 桶**——那是跨账号共享的，清了等于给爆破地址发新额度。
Redis 出错时**回 500 而不是静默 200**：管理员会以为解锁了而用户仍登不进去。
新权限码不给 `system:user:update` 顺带放行——后者是日常高频操作，几乎必然授给管理员，
而"解锁"意味着"我确认这个人是本人"，两者共用开关就等于让前者必然带出后者。

**在线会话** — 登录成功时登记 `sess:*`，`GET /api/admin/users/{id}/sessions` 列举，
`POST .../sessions/{jti}/revoke` 单吊销 + 新权限码 `system:session:manage`。

- 存储增长靠 **JWT 自身 TTL 自过期**。`revoke_user_sessions`（`user_revoked_before`）是整用户粒度，
  单会话吊销仍走 jti 黑名单——两条路径不能混。
- 登记只用于管理视图，缺一条记录不影响认证结论，但会让"这个人在哪些设备上登录"漏掉一次登录，
  而管理员正是靠这个列表判断账号是否被盗用。所以**登记失败不放行登录**。
- jti 会直接拼进 Redis 键，列举用的是 `SCAN sess:{user_id}:*` 这个 glob——
  不校验 UUID 形状的话，一次键名污染就能让列表凭空多出别人的会话。

**`POST /api/admin/users/import`**（CSV）— 必需列 `username,email,password,roles`，
可选 `display_name`，`roles` 单元格用 `|` 分隔。

- **逐行成败**，失败带**行号**（含表头上限）。"3 行失败"等于让管理员自己数行号。
- **授权下界整批前置校验**：整批能授出的角色先判，越权则一行都不写。
- 试运行（`dry_run`）**绝不落库**，但仍报告会有几行成功——管理员需要预览才能决定返工。
- **口令不入审计**：`params` 与 `result` 两列都要查，只查 `result` 是不够的。

### 🐛 审计 `action` 列宽 100，超长路径的审计整条静默消失

`audit_logs.action` 是 `VARCHAR(100)`，而 `path` 是 `VARCHAR(500)`——
两列装的是同一段信息（`action = "{method} {path}"`），宽度却差 5 倍。
新端点 `POST /api/admin/users/{id}/sessions/{jti}/revoke` 第一次把路径推过了 100 字符这条线：
拼出来 107 字符，INSERT 直接失败；而中间件在 `tokio::spawn` 里写库，失败只留一行 `tracing::warn!`，
**请求照常返回 200**。表现是"这个操作没有审计记录"，而不是"审计写不进去"。

迁移 015 把 `action` 拓宽到 `VARCHAR(512)`，中间件再加 `MAX_ACTION_LEN` 截断。
**改列宽而不是只截断**：action 的唯一用途就是检索，一条被截掉尾部的 action 检索不到，等于没有。

### 🐛 一个夹具泄漏，表现为"随机大面积 403"

全量集成测试首跑 **53 条红**，几乎全是 `缺少权限：system:user:create`；而**单条跑全绿**。逐层查到：

1. `the_user_list_can_be_filtered_by_role_and_by_active_status` 造 3 个普通账号加 1 个
   **持 admin 角色的账号**，测完只清理了 operator 夹具，**这 4 个一个都没删**。
2. `ensure_not_last_admin` 判的是 `count_users_with_role("admin") <= 1`——**全库**计数。
   多一个 admin 账号，守卫就认为"还有别人是 admin"而**放行**降级，
   `last_admin_cannot_be_demoted_or_deleted` 于是把真 admin 降掉。
3. 之后**每一条**用例都因 admin 掉权而红。

守卫本身没坏，**它的成立前提被夹具破坏了**，而这个前提从未写进任何测试。
修法：两条筛选测试补齐账号清理；`last_admin_cannot_be_demoted_or_deleted`
**开头先查全库 admin 列表并断言恰好是 `["admin"]`**，失败信息直接点名"是某个夹具没清理"。

与 v0.19.0 那次"19 处夹具堆 212 个角色"同源：**夹具泄漏的症状可以离病因一百多条用例**。
`opf_*` 前缀守卫只圈 operator 夹具，圈不到藏在业务用例里的那种。

### 🐛 两处与既有守卫冲突的新端点

**头像端点的入参错误绕过统一信封。** `Multipart` 提取器在 Content-Type 不对时
**在进入处理函数之前**返回 `text/plain` 的 400，于是同一个"入参不对"有两种形态，
调用方没法只靠 `code` 分支处理。改接 `Result<Multipart, MultipartRejection>` 自己翻译。

**`GET /users/{id}/sessions` 对不存在的用户回 404。** 而 `GET /users/{id}/roles` 对同一个
`{id}` 回 200 + 空数组——同一个"查这个人的附属信息"，两种相反的答案。
仓库对读端点的统一约定也是"不存在即空"。改为返回空数组，并把代价写进注释：
写错或已删除的 id 与"这个人确实没在线"确实分不开。

### 缺陷注入验证（每处都做了，会红并回滚）

| 注入 | 结果 |
|---|---|
| `audit_logs.action` 列宽改回 `VARCHAR(100)` | `every_write_operation_leaves_an_answerable_change_summary` 红在"审计里查不到 revoke 的摘要"——**正是原症状**：请求 200 而审计整条消失 |
| 解锁的 scope 去掉 email 桶 | `unlocking_clears_the_email_counter_too_not_just_the_username_one` 红：`left: 3, right: 0` |
| profile 的清空 flag 退回"内层为 Some 才写" | `a_user_can_set_and_clear_their_own_display_name` 红：`清空后应为 null, left: String("张三")` |
| 头像端点退回提取器默认拒绝 | `a_non_multipart_avatar_upload_still_returns_the_json_envelope` panic（走不到信封断言） |
| sessions 端点加回 `find_by_id` 的 404 | `sessions_of_an_unknown_user_are_an_empty_list_not_a_404` 红：`left: 404, right: 200` |

每次注入后回滚并查库清残留（`opf_*` / `filt_*` / `countme_*` / `avct_*` 均为 0，
admin 的 `user_roles` 保持 `admin`）。

### 部署注意

头像落在 `UPLOAD_DIR`（默认 `./uploads`）。**容器部署必须挂卷**：
`docker-compose.yml` 已加 `uploads:/app/uploads` 命名卷，`Dockerfile` 在镜像里就
`mkdir -p /app/uploads/avatars` 并设好属主——卷挂到镜像里**已存在**的目录会继承属主，
挂到不存在的路径则按 root 建，容器内的非 root 用户写不进去，头像上传在运行时才报错。

### 门禁结果（全绿）

`cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` 0 warning /
92 单元 / **162 集成 passed** / `pnpm lint` 0 errors（1 既存 warning）/ `pnpm typecheck` /
`pnpm test` **161 passed** / `pnpm build`。

---

## [0.19.0] - 2026-10-03

主题：**承重守卫生效范围太窄，于是烂掉的端点没人发现。**

`GET /api/admin/export/users` 自 v0.11.0 起**每个调用都 500**，跨越七个版本无人知晓。

### 缺口表

| 缺口 | 实测依据 |
|---|---|
| `export/users` 每个调用都 500 | 迁移 010 新增 `must_change_password`，而 `controller/demo.rs` 的裸 SQL 手写列名漏了它，`sqlx::FromRow` 运行时找不到该列。实测修复前 500、修复后 200 并返回 23555 字节真 xlsx |
| 根因是守卫生效范围只覆盖写端点 | `every_documented_write_operation_is_covered_by_the_audit_test` 从 OpenAPI 派生的只有 POST/PUT/DELETE。**这是个 GET 端点，文档里 22 个 GET 一个都不在它视野里** |
| 用户名大小写可冒充 admin | 用户名是**登录键**，也是管理员在列表里辨认账号的依据，而 `UNIQUE(username)` **大小写敏感**。`Admin` / `ADMIN` / `aDmIn` 可与真 admin 并存，且**自助注册一次就能造出来** |
| 测试夹具在共享库里堆垃圾 | `operator_with_codes` 被 19 个用例共用，返回的 `(token, role_id, user_id)` 无一被清理。实测累积 212 个角色 / 424 个账号（整库 270 角色 / 424 用户），而 142 个用例全绿 |

### 修法一：列名单一数据源 + 守卫生效范围扩到全部端点

`repository::user::USER_COLUMNS` 集中列名（照 `repository::menu::MENU_COLUMNS` 的形状）。
原先 **10 处手写**：user.rs 8 处 + demo.rs + rbac.rs，全部改用它。

新守卫 `every_documented_endpoint_is_reachable_without_a_server_error`：从文档派生**全部 50 个端点**。
GET 必须 2xx（无必填体，4xx 也是缺陷）；非 GET 只要不是 5xx（发 `{}` 停在校验层，不改真实数据）。
路径与必填 query 参数同样由文档派生，**新增端点自动进探针表**，不用手动登记。

### 修法二：用户名 / 邮箱大小写归一

写入侧三条路径全部归一（trim + 小写），**先归一再校验**，保证被校验的就是被存下的。
查重因此自动变成"归一后比较"，不必再单独写一条大小写不敏感的查重——两处规则迟早会走偏。
登录侧复用同一个归一值走查库 / 限流 / 审计三处。

迁移 013 遇到冲突**报错让应用起不来**，不照抄 008 的"跳过"：跳过会让那行变成
**登录不到的孤儿账号**——归一后所有写入都是小写，`find_by_username("admin")` 只命中小写那行，
旧 `Admin` 行再也登不进去，却仍挂在库里、仍持原角色，且在列表里与真 admin **肉眼无法区分**。
那比迁移前更糟。合并会丢权限（`user_roles` 按 `user_id` 关联），必须管理员显式决定。

### 顺带修掉 013 自己引入的缺陷

仓储按约束名翻 409 时只认迁移 001 的 `users_username_key`，而**仅大小写不同的插入
撞的是新建的 `users_username_lower_key`**（已实测）。只认旧名字的话，013 注释里承诺的
"第二道防线"会报 **500 而不是 409**——防线拦住了却报 500，等于把一个可诊断的冲突
变成"用户说系统坏了"。这条路径走 HTTP **永远测不到**（应用侧查重会先一步挡住），
所以测试直接调 `UserRepository::create` 断言拿到 `AppError::Conflict`。

### 修法三：测试夹具不留残

`cleanup_operator` 走 API 而非 SQL，**顺序是先删用户后删角色**：删角色时"仍有 N 个用户使用该角色"
会回 400，这条拒绝是有意设计——`user_roles` 的 ON DELETE CASCADE 会静默剥掉用户的角色，
让人变成"没有任何角色"的用户而不自知。清理要顺着它的意思，而不是绕开它。

守卫 `the_permission_code_fixtures_leave_no_holder_behind` 的范围从 `granted_temp_button`
一支扩到两支。关键前置是给夹具加 `OPERATOR_FIXTURE_PREFIX = "opf_"`：
不加就没法精确圈定（原命名与 `grantee_role_*`、`strong_role_*` 撞形状，
守卫要么长期误报、要么被人加豁免，两者都等于没有守卫）。

顺带清掉历史累积 181 角色 + 181 账号，库里角色数从 270 降到 87。

### 缺陷注入验证（每处都做了，会红并回滚）

| 注入 | 结果 |
|---|---|
| `demo.rs` 改回漏列的手写版 | 守卫当场红并**点名该端点** |
| 抽掉 19 处夹具清理中的 1 处 | 对应测试**仍然通过**（泄漏是静默的），守卫红并点名 `opf_hr_role_c634c48b` / `opf_hr_user_d7b5b3b4` |
| 三个归一函数全退回 `raw.to_string()` | 2 条集成测试红 |
| **只去掉 `.to_lowercase()`，保留 trim** | 2 条红，且**红在大小写断言上**（隔离出"大小写"这一个属性） |
| 约束名只认 `*_key` | `the_database_rejects_identities_differing_only_in_case` 红：`InternalServerError(... "users_username_lower_key")` |

第一次注入只去掉小写时测试红在**别的地方**（trim 那一关），说明"大小写"这个属性
其实没被单独覆盖，于是做了第二次更精确的注入。

### 门禁结果（全绿）

`cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` 0 warning /
81 单元 / **133 集成 passed** / `pnpm lint` 0 errors / `pnpm typecheck` / `pnpm test` **161 passed** / `pnpm build`。
CI run `37132068490` 三 job 全绿。

---

## [0.18.0] - 2026-10-03

主题：**创建账号的两个入口，规则停在七版之前。**

v0.11.0 把口令策略从"至少 6 位"收紧为"至少 8 位 + 两类字符"，并新建了
`utils/password.ts` 与一条把它绑到后端的契约测试。**但那次只迁移了改密页**：
注册页与管理员建号对话框仍留着 v0.10 时代写下的 `min: 6`，整整七版没人碰。

契约测试只把 `password.ts` 绑到了后端，**没有任何东西把"页面"绑到 `password.ts`**——
这正是它能漂移七版的原因。

### 缺口表

| 缺口 | 实测依据 |
|---|---|
| 注册页教人用 6 位口令 | 用户按提示填 `abcdefgh` 能过前端校验，填完整个表单才收到 `400 密码复杂度不足` |
| 管理员建号对话框教人用 6 位口令 | 同上，规则写在 `system/user/index.vue` 里 |
| 建号对话框**完全没有**用户名字符集规则 | 管理员输入 `user@name` 能过校验，保存后才收到"用户名只能包含字母、数字、下划线和连字符" |
| 登录页把合法邮箱挡在门外 | 该字段收"用户名**或**邮箱"，却写死 `max: 50`；`users.email` 是 `varchar(255)`。实测 88 字符邮箱注册与登录都成功，但输入框根本敲不进第 51 个字符之后 |
| 用户名长度按**字节**判定 | 17 个汉字（51 字节 / 17 字符）被拒，报错文案说的是"3-50 个**字符**"。而 `users.username` 是 `varchar(50)`，Postgres 按字符计，列本身收得下——字节制凭空砍掉三分之二的中文用户名容量 |
| 对外 OpenAPI 文档也在教 6 位口令 | `GET /api/openapi.json` 实测：`RegisterRequest.password` 的描述至今是"密码（至少 6 个字符）"。按它写出来的调用方会被真实接口 400 拒掉 |

### 修法一：共享校验器（`frontend/src/utils/accountRules.ts`，新）

把用户名 / 邮箱 / 口令 / 登录标识符四组规则收敛到一处，注册页、管理员建号、登录页
全部改为引用。补上此前不存在的绑定：**页面用共享校验器**，由
`account_forms_use_the_shared_validator` 这条 Rust 契约测试守着。

几处刻意的不对称，都写在文件注释里：

- 用户名字符集用 `/^[\p{L}\p{N}_-]+$/u` 而非 `[a-zA-Z0-9]`。后端是 Rust 的
  `char::is_alphanumeric()`，Unicode 感知；写成 ASCII 白名单会让中文用户名
  被前端拦下而后端放行——漂移方向只是反过来，缺陷照旧
- 登录标识符**刻意不套** `usernameRules`：它收的是用户名**或**邮箱，
  邮箱含 `@`，会被字符集规则拒掉。上限取 255 而非 50，与 `users.email` 对齐
- 登录口令**刻意只判空**：后端登录验 Argon2 哈希，不套明文复杂度策略。
  原先那条 `min: 6` 方向上是宽松的、没锁死谁，但它没有任何依据，
  注释里却写着"对齐后端"

### 修法二：长度按字符计（`src/utils/validation.rs`）

`validate_username` 从 `username.len()`（字节）改为 `chars().count()`，
与同文件的 `validate_password` 同一把尺子；并把 `3` / `50` 提成
`USERNAME_MIN_LEN` / `USERNAME_MAX_LEN`，让报错文案也从常量取值——
原文案硬编码 `"3-50"`，已经和判据脱钩过一次（判据是字节、文案说字符）。

### 修法三：对外文档说实话（`src/model/user.rs`）

`RegisterRequest` 的字段注释修正。这不是新增限制，只是让 `utoipa` 导出的
OpenAPI 不再教人用已被拒绝的规则。

### 顺带修掉：一条靠运气通过的假阳性守卫

上一轮跑完整集成组暴露两条红（角色列表 `total == items.len()` 与
`page_size` 上限 200、测试库累积 220+ 角色冲突），改成真正的分页不变量
`items.len() == min(total, page_size)` 并新增 `all_role_ids()` / `find_role_row_by_id()`
两个助手后已绿。

但注入验证时发现那条守卫**本身仍是假阳性**：去掉 `ORDER BY r.id ASC` 后，
只有手工把全表 `created_at` 改成同值它才会红——能不能变红取决于
"库里数据恰好不并列"，那是运气不是断言。三处修法：

1. 测试自己造并列（建 5 个角色后统一 `created_at='2020-01-01'`，
   页长 2 时并列组横跨两个页边界，组内次序一抖必然重复或缺失）
2. 翻页总数与 `total` 独立对账，否则两种页长可能**同样**漏读而一起假绿
3. 先清理再断言——断言恰恰在"分页真有 bug"时失败，清理写在之后等于永不执行

注入验证：去掉 `id ASC` 后只取回 163/218 条、大量重复 id，测试红，
且失败后 `pgrole*` 残留为 0。

### 顺带修掉：e2e 会整轮挂死，以及一条永久假红的套件

**失败套件永不退出**。`waitFor` 超时是**故意**抛错的，但抛错后套件末尾的
`s.stop()` 就执行不到，CDP 的 WebSocket 仍开着，Node 事件循环永不退出；
而 `run.mjs` 用同步 spawn——**一个失败的套件让整轮 e2e 永久卡住**。
缺陷注入时尤其致命：本该看到一条 FAIL，实际是永远转圈的终端。
两道防线：`harness.mjs` 装 `uncaughtException` / `unhandledRejection` 退出守卫，
`run.mjs` 给每个套件加硬超时（默认 300s，`E2E_SUITE_TIMEOUT_MS` 可覆盖）。

**`v016` 中途失败后永久不可重跑**。它的 `CODE = 'custom_type'` 是固定值
（demo 页要读它），而清理只写在末尾：断言一 panic，`seed()` 撞唯一约束
`dict_types_code_key` 直接 500，**一次中途失败被放大成永久性假红**。
改为按 code 幂等清理，开头也调一次（删类型会级联删字典项）。

### 门禁

`cargo fmt` / `clippy -D warnings`（零警告）/ `--lib` 77 条 /
集成非 ignored 12 条 / 集成 `--ignored --test-threads=1` **129 条全绿** /
前端 lint（0 error）/ typecheck / vitest 155 条 / build /
e2e **11 个套件全过**，其中 `v018-account-form-truth.mjs` 19 条。

## [0.17.0] - 2026-10-03

主题：**菜单树能成环，然后整个服务就没了。**

上一版摸到"菜单成环后从界面上静默消失"。往下再挖两层，破坏等级从
"一个功能坏了"升级成**全站不可用且界面无法自救**。

### 缺口表

| 缺口 | 实测依据 |
|---|---|
| 菜单树能成环 | `PUT /api/admin/menus/{A} parent_id={B}`（B 是 A 的子节点）→ **200**，写入成功。`update_menu` 对 `parent_id` 不做任何校验，数据库外键只挡"指向不存在的菜单"，**挡不住环** |
| 成环后整棵子树静默消失 | `build_tree` 判根看"父节点是否在集合内"，环上无一为根 → 整支被剪掉。侧栏与管理页是同一棵 `build_tree`，所以管理员在界面上**也看不到它**，无法点开改回来。审计还记成一次正常成功 |
| 连"删掉它"这条自救路都被堵死 | `granted_codes_in_subtree` 用 `WITH RECURSIVE ... UNION ALL` 走子树，不去重 → **永不收敛**。实测 `DELETE /api/admin/menus/{环上节点}` 客户端 10s 超时、无响应、连接不归还 |
| 整个 API 服务不可用 | 池上限默认 20 且代码里没有 `statement_timeout`。28 节点大环 + 22 个并发 DELETE → 19 个挂起、3 个 500，库内 21 条 active 持续增长。此时**与菜单毫无关系**的 `GET /api/admin/users` 也 500（10s 超时） |
| 前端静默改结构 | `openEdit` 不重置 `parentId`，弹窗里又**根本没有上级字段**。先在 A 点"新增子菜单"、再点 B 点"编辑"保存，B 被静默挂到 A 下，界面上没有任何提示 |
| 无法把菜单摘成根 | `parent_id: Option<Uuid>` 让"没传"与"传 null"不可区分。实测 `{"parent_id": null}` 返回 **200**、字段原样回显，而库里父级纹丝不动。UI+API 组合下管理员**没有任何途径调整菜单层级** |
| 移动菜单不留审计 | 改 `parent_id` 是结构性变更，审计里只有一行"无权限码变更" |

### 修法一：挂载校验（挡住环进来）

`ensure_attachable` 拦三种情况：父级必须存在（给可操作消息，而不是让 FK 抛 500）、
非自身、不在自己子树内。`create` / `update` 双路径都接上。

**刻意在 Rust 侧向上走祖先链、不用 SQL 递归**，并带 `visited` 集合：
库里**可能已经存在环**（历史脏数据、运维直连写入），任何无 visited 的遍历
在那种数据上自己就会死循环。

只在父级**真的变更**时校验。"父级没变"也校验会顺手把改名改图标这类无害操作堵死，
包括"把环上的节点摘成根"这条唯一的自救操作。

### 修法二：`UNION` 而非 `UNION ALL`（保证挂不死）

递归 CTE 改用 `UNION` 按 `id` 去重，环上转一圈就停，在无环数据上与 `UNION ALL` 语义一致。
第一道防线让环进不来，这里是第二道防线。

### 修法三：`parent_id` 三态化

`Option<Option<Uuid>>` + 自定义反序列化器区分三种情况：
`None`=本次不改 / `Some(None)`=摘成根 / `Some(Some(id))`=改父级。

### 修法四：结构可见 + 可自救

新增 `GET /api/admin/menus/diagnostics`，报出走不到根、因而不在任何菜单树里的节点
及其原因（成环 / 悬空引用）。菜单页顶部告警条列出它们并提供"摘成根菜单"。

修复动作**复用既有公开 API**（`PUT .../menus/:id` + `parent_id: null`），
不新增第二条写路径——那又多一处需要同样权限守卫的地方。

刻意**不改** `build_tree` 的"父不在集合内即视为根"语义：它在按角色过滤时是必需且正确的。

### 附带：移动菜单进审计

记录"上级从 X 改为 Y"或"摘成根菜单"，两侧都用菜单名而非裸 UUID。

### 顺带修掉的既有缺陷

- **测试清理助手自身会在环上挂死**：`cleanup_temp_menu_dir` 内部也是 `UNION ALL` 递归。
  一旦某个用例失败跳过清理，残留环会让后续清理永久挂起。已改 `UNION`，
  并新增 `flatten_menus_to_roots` 在清理前剪环。
- **v0.14 e2e 套件的断言是状态依赖的假红**：`v014-retention-honesty.mjs` 用
  "alert 里含'早于'两个字"来判定范围提示，但**清理横幅**在跑过清理任务后
  也含"早于"。v0.16.0 门禁时是绿的，库里一旦有了清理记录就变红。
  已改为锚定范围提示自己的文案。

### 收尾时才发现的：工具自己在撒谎

提交前核对探针残留，发现授权探针报告"探针夹具已全部清理"而库里躺着 6 条残留。
顺着查下去，它有三处缺陷叠在一起——**每一处都是让"绿灯"失去意义的假绿**：

| 缺陷 | 后果 |
|---|---|
| 清理顺序直接删角色/菜单 | 探针自造的 `probe:<uniq>:<n>` 是全新权限码，admin 按设计不持有它（种子"只授权新建行"）。于是删除撞上 v0.8.0/v0.9.0 的授权下界必然 403，而探针**照旧报告清理完成**。出路是先收回授权（降权方向不设限）再删 |
| 清理断言写反了 | `created.*.length > 0` 判的是"我建过东西"，标签却写着"已全部清理" |
| 数据侧断言比的是一个从不存在的人 | `POST /api/admin/users` 的 body 用 `ctx.name()`、verify 查 `ctx.pending()`——两个不同名字。断言永远查不到，恒真 |

第三条尤其要命：它正是探针最核心的那条断言（建号时的授权天花板）。
**一个恒真的断言比没有断言更坏**——它让人以为这条不变量有人守着。
实测坐实了鉴别力：同一个查找逻辑，指向 body 真正用的名字时能命中（会红），
指向 `ctx.pending()` 时恒不命中（永不红）。

修法：清理改成"删用户 → 收回授权 → 删角色 → 删菜单"；断言改为**按名字回查数据库**
而不是查账本（账本只能证明"请求发出去了"，证明不了删除成功——403/400 会被
`callApi` 静静吞掉）；`ctx.pending()` 这套只记名字不记 id 的旁路改为进清理账本。

探针自造的动态权限码**不是产品的死角**，这条路已验证走得通：
`PUT /roles/{id}/menus` 清空授权后再删即放行。

### 顺带修掉的第二笔账：集成测试的夹具泄漏

`granted_temp_button` 每跑一次就把一个持有者角色和一个持有者账号留在库里
（临时菜单目录有 `cleanup_temp_menu_dir` 收拾，这两个没有），
而 126 个用例**全绿**。返回值还只给了 uid、没给角色 id，调用方即使想清也清不了——
元组允许 `_holder_uid` 把它随手丢掉，一丢就再也找不回来。

改为返回具名结构体（字段摆在那里，丢掉会显眼），补 `cleanup_holder`，
并新增 `the_permission_code_fixtures_leave_no_holder_behind` 把"漏清理"变成红灯。

**刻意不扩大范围**：`operator_with_codes` 造的 20 处操作员角色同样没人清理
（累计已 156 个角色 / 264 个账号），那是另一笔账。范围一旦放大，这条守卫就会
一直红——**一个长期红的守卫等于没有守卫**，不如先守住已经修干净的那一半。

### 验证

| 项 | 结果 |
|---|---|
| 后端单测 | ✅ 75（+1：三态反序列化） |
| 集成（非 ignored 组） | ✅ 8 |
| 集成（`--ignored` 组，真实 PG+Redis） | ✅ 126（+10） |
| 前端 vitest | ✅ 144（+7） |
| e2e | ✅ 10/10 套件（新增 `v017-menu-hierarchy` 26 条断言） |
| 授权探针 | ✅ 41/41 |
| 缺陷注入 | 8 处：CTE 改回 `UNION ALL`、拆掉 `ensure_attachable`、还原 `.or()` 语义、前端去掉子树过滤、前端还原 `openEdit` 不重置；
探针去掉"先收回授权"、探针不登记放行侧账号、集成测试去掉 `cleanup_holder`
—— **每一处都有对应用例变红** |

集成测试断言一律落在**外部可观测后果**上（HTTP 状态、树里还在不在、
库里父级是什么、请求会不会返回），而不是"某个私有函数返回了 Err"。
其中一条专门查库而非看响应体——v0.16.0 最恶劣的症状正是响应体原样回显了没生效的值。

## [0.16.0] - 2026-10-03

主题：**字典的三个开关都是摆设。**

字典模块自 v0.4.0 之后没被任何版本正面处理过（856 行 Rust + 286 行 Vue），
而 117 条集成测试里**没有一条碰字典**。逐个端点真实 HTTP 打了一遍，
发现三处管理员"以为自己能控制、实际控制不了"的东西，外加一处让整页空白。

### 缺口表

| 缺口 | 实测依据 |
|---|---|
| `status=disabled` 完全不生效 | 禁用字典项后 `GET /api/dict/sys_yesno/items` 仍返回它；禁用**类型**后读取端仍返回 3 条。`DictSelect` 直接用该端点渲染下拉框，于是管理页写着"禁用"的选项，在所有业务页面的下拉框里照常出现 |
| `is_default` 可以有任意多个 | 连续创建 A(is_default=true)、B(is_default=true) → 均 200 → 两个都是 `True`。没有任何约束 |
| 「刷新缓存」刷新不了任何东西 | 塞入陈旧数据后点按钮，返回 `{"data":"缓存刷新成功"}`，再读**仍是陈旧数据**，Redis 里键也没动。审计还记着"刷新字典缓存，共 1 个类型" |
| 字典管理页整个是空白的 | 页面用 `#left`/`#right` 命名插槽，而 naive-ui 的 `n-split` 只认编号插槽 `#1`/`#2` → 两个 pane 全空。菜单点得进去、接口全部正常，所以只看接口永远发现不了 |

前三处是同一件事的三个面：模块给了管理员三个控制，每个在界面上都正常工作、
写入也都返回 200，但**对实际行为没有任何影响**。写入成功 ≠ 生效。

### 修法一：`status` 真的生效

新增 `list_enabled_items()`，读取端点改走它；类型被禁用时整份字典返回空。

一个容易漏的细节：**禁用类型时的空结果不能写进缓存**。否则管理员重新启用后，
读到的还是那份"空"，要等一小时 TTL 才能看到自己刚做的修改。

`list_items()`（管理页用）保持不过滤——管理页必须能看到禁用项才改得回来。

### 修法二：`is_default` 靠 DB 兜底

`create_item` / `update_item` 改为事务：设默认前先
`UPDATE ... SET is_default=FALSE WHERE dict_type_id=$1 AND is_default`。
放在同一事务里是因为分两次写的话，并发请求会各自看到"现在还没有默认项"，最后写两个。

迁移 `012_dict_item_single_default.sql` 先清理既有违规（每组保留 `created_at` 最早的一个），
再建部分唯一索引 `idx_dict_items_single_default ON dict_items(dict_type_id) WHERE is_default`。

索引冲突映射成 409 而不是 500（沿用 `repository/menu.rs` 的 `map_write_violation` 套路）——
并发下唯一索引兜底触发时，界面应当说"资源冲突"，不该说"服务器内部错误"。

顺带堵上一个组合坑：**禁用项不能设默认**。否则会出现"默认项指向一个业务页面看不到的值"，
表单里看不见却默认选中——比没有默认值更难排查。

### 修法三：「刷新缓存」真的清

新增 `RedisClient::delete_by_prefix()`（SCAN 游标循环 + 批量 DEL，每轮判 `cursor == 0`）。
`refresh_cache` 改为：真删 → 只回填 `status=enabled` 的类型。

返回值从 `"缓存刷新成功"` 改为
`DictCacheRefresh { cleared_keys, reloaded_types, skipped_disabled_types }`，
前端 `buildRefreshMessage()` 如实转述，并且**把 0 单独说出来**：
"没有需要清理的缓存键"和"已清空 3 个缓存键"如果都报同一句"成功"，
管理员无法区分"确实清了"与"什么都没发生"。

审计也跟着记实际数量，而不是"刷新字典缓存"。

### 修法四：字典管理页不再空白

`#left`/`#right` → `#1`/`#2`，并在代码里写明为什么
（写错时 naive-ui 不报错，只是静默渲染成空）。

### 附带修掉：导出 Excel 会 500（跑全量 e2e 才暴露）

`v013` 套件在跑全量时红了：`GET /api/admin/logs/audit/export` 返回 500。
后端日志是一段 panic：

```
thread 'tokio-rt-worker' panicked at rust_xlsxwriter-0.82.0/src/xmlwriter.rs:291:
byte index 28 is not a char boundary; it is inside '（' (bytes 27..30)
of `新建用户 "prbo6hpb_x20"（67d7a0e0-...），角色：prbo6hpb_strong`
```

触发条件很容易凑齐：审计摘要里只要有 `_x`，且它后面 4 个字节内有中文，
库里的 `escape_xml_escapes` 就会按字节切片 `original[index+2..index+6]` 而 panic。
而审计摘要本来就有全角括号（`新建用户 "..."（uuid），角色：...`），
用户名又是管理员自己填的——**任何一个含 `_x` 的名字都会让整个审计导出永久 500**。

这是既有问题（`src/controller/user.rs` 的摘要格式早于本版），
但只有**全量按序**跑 e2e 才会撞上：`role-assignment-guard` 先造出 `*_x20` 的用户，
后面的 `v013` 再导出就炸。所以此前"门禁全绿"里没有它。

修法：`rust_xlsxwriter` 0.82 → 0.99.1。库自己的转义逻辑会在非字符边界切片，
升级后这条路径已修正。已验证导出内容**逐字未变**
（`删除字典类型 "probe_x"（0c292627-...）` 原样出现在 sharedStrings 里，
没有被转义成 `probe_x005F_x`）。本项目只用到 `Workbook` / `Worksheet` /
`Format` / `write_string` 这几个稳定 API，升级零改动。

### 缺陷注入

四处修复逐个反向破坏，确认对应用例会红：

| 注入 | 结果 |
|---|---|
| 读取端不过滤 disabled **项** | ✅ 被抓 |
| 读取端不过滤 disabled **类型** | ✅ 被抓 |
| `create_item` 不取消旧默认项 | ⚠️ **第一轮没被抓**——原有那条用例只走 PUT（update 路径），POST（create 路径）零覆盖。补了 `creating_a_default_item_clears_the_previous_one` 后被抓（且暴露 500→409 映射缺失） |
| `refresh_cache` 退回 `for dt { get_dict_by_code() }` | ✅ 两条 refresh 用例同时变红 |
| `buildRefreshMessage` 退回无条件"成功" | ✅ 前端单测变红 |
| `#1`/`#2` 退回 `#left`/`#right` | ✅ 新增的 3 条空白页守卫变红，其余 21 条仍通过（证明守卫是针对性的） |

### 验证

集成测试 6 条字典用例 + 前端单测 4 条 + e2e 套件 24 条断言。
e2e 覆盖到**真实下拉框**里禁用项消失，而不是只验接口返回值。

全门禁：后端单测 74 + 集成 124（8 非 ignored + 116 ignored）、
前端 137（16 个文件）、e2e 9 套件全绿（`v013` 由 18/21 变 25/25）、
授权探针 41/41。

## [0.15.0] - 2026-10-03

主题：**会话失效时，界面说的是"404 页面未找到"。**

令牌被服务端吊销后——改角色、改密、停用账号都会吊销——页面上的每个接口
都返回 401。此前 401 只有一个"弹一句'未授权，请重新登录'"的动作，
于是用户看到的是这样一幅界面：

```
logout -> 200
同一令牌 /api/auth/me -> 401 {"message":"令牌已被注销，请重新登录"}
goto /system/user -> path=/system/user（没跳）
正文 = "404 页面未找到 返回首页 未授权，请重新登录 ×4"
localStorage 令牌 = 有
```

### 缺口表

| 缺口 | 实测依据 |
|---|---|
| 不跳登录页，只弹一句 | `path=/system/user`，用户被留在一个再也点不动任何按钮的页面上 |
| 令牌留在 localStorage | 之后每次进页面重演一遍，包括刷新 |
| 页面写着"404 页面未找到" | 会话没了 ≠ 页面不存在。这是**方向性相反**的诊断：按 404 排查会去找根本不存在的路由 |
| 同一句提示弹 4 次 | `menuStore.load()` 与 `permissionsStore.load()` 是 `Promise.all` 并发 → 2 个 401；每个又被 store 的 `handleError` **再弹一次** |
| 输错口令被告知"未授权" | 后端原话是 `"凭证错误: 用户名或密码错误"`，前端覆盖成了"未授权，请重新登录"——说的是另一件事 |
| 点"退出登录"被告知"会话已失效" | 吊销后再调 `/auth/logout` 会 401。他**正是**主动结束会话的人，因果被说反了 |

### 修法一：401 单点出口 `utils/session.ts`

清令牌 + 清 GET 缓存 + 跳登录页，并把**后端原话**经 `sessionStorage` 带过去。
前端不另编一套通用文案——后端分得清是哪一种 401（口令错 / 被吊销 / 无效过期），
前端编一句就把这个区分抹掉了。

用 `sessionStorage` 而非 `localStorage`：这是本次标签页的临时状态，
关掉标签页就该消失，而"记住密码"的用户名必须留在 `localStorage`。
守卫此前那句 `localStorage.clear()` 会连用户名一起抹掉。

跳转回调由 `router/index.ts` 在模块加载时注册进来，而不是让 `utils/session.ts`
直接 import 路由——那样会形成 `router → stores → api → session → router` 的循环依赖。
与 `utils/message.ts` 的 `registerGlobalApis` 同一套路。

### 修法二：豁免清单按"这个 401 有没有正常用户语义"定

排除 `/auth/login`（输错口令时用户正站在登录页上等他改）与 `/auth/logout`
（点退出的人正是主动结束会话的人）。其余 401 走出口。

匹配用**后缀**而非全等：调用点万一写成完整路径（`/api/auth/login`）时，
全等匹配会漏掉，漏掉的后果是**输错口令被弹去"会话已失效"**，
登录页从此无法正常报错。这是安全方向的失败，宁可多匹配。

### 修法三：`handleError` 不再二次弹窗

`ApiError.reported` 标记这条错误是否已由响应拦截器展示过。
这是**全站性**缺陷，与 401 无关——任何走 `handleError` 的失败都弹两次。
因此修在 `handleError`，而不是加一条 401 去重补丁。

`ApiError` 同时带上状态码与**后端原话**：此前抛出的 `new Error(message)`
既没有状态码，message 也是 `"请求失败 (401)"`。监控页等界面直接显示
`error.message`，等于拿状态码当解释。

### 修法四：守卫中止导航，而不是落到 404 兜底

守卫是回调式的（`guard.length === 3`），`return` 会让整条导航永远悬着、
页面卡在空白——必须是 `next(false)`。也不能 `next('/login')`：跳转已由出口
发起，再 redirect 一次是第二次导航（多半撞上重复导航失败）。

### 途中修掉的三个自己的 bug

| 问题 | 怎么发现的 |
|---|---|
| 守卫 401 分支直接 `return` | 导航悬着、页面空白 |
| 登录页文案重复成"请重新登录，请重新登录" | 截图复核 |
| 抛出的错误消息是 `"请求失败 (401)"` | 缺陷注入时从监控页界面上看到 |

### 缺陷注入

| 注入 | 结果 |
|---|---|
| A：401 不走会话失效出口 | ✅ 被抓，6 条红 |
| B：把 `/auth/login` 移出豁免清单 | ✅ 被抓，2 条红 |
| C：`handleError` 恢复二次弹窗 | ✅ 被抓，2 条红 |
| D：守卫 401 改成 `next()` | ✅ 被抓（新增守卫单测后） |

注入 D 最初**没被抓**，e2e 27 条全绿。原因是跳转由拦截器注册的 handler 驱动
（`router.replace('/login')` 立即改写 `pendingLocation`，vue-router 随即取消原导航），
守卫分支在当前接线方式下属于**纵深防御而非承重**。这是实测结论，不是推测。
已补 `src/router/__tests__/authGuard.spec.ts` 直接观察守卫传了哪个 `next`——
handler 的注册时机一旦变化（某个入口忘了 import `router`），
此刻没人拦住的 404 兜底就会回来。

补那条单测时踩到的两个坑同样写进了注释：`vitest.config.ts` 原本没注册
`@vitejs/plugin-vue`，导航一旦成立就在 import-analysis 阶段炸掉；
以及 vue-router 对**同一目标**的第二次 `push` 直接判定 duplicated 并跳过守卫，
让"过期令牌"那条断言空过。

### 门禁

前端单测 **130**（新增 25 条）/ typecheck / lint / build /
e2e **27/27** / 后端零改动（fmt·clippy·单测·集成全跑，无回归）

README 补上 `METRICS_FLUSH_INTERVAL_SECONDS` / `METRICS_KEY_TTL_SECONDS` /
`METRICS_MAX_BUFFERED_ENDPOINTS` 三个变量（一直存在于 `config/mod.rs`，
文档里却没有）。不为它们造测试——那是自证。

## [0.14.0] - 2026-10-03

主题：**审计会过期，但没人被告知。**

v0.13.0 把「改了什么」唯一地存进 `audit_logs.result`，而 CHANGELOG 当时
自己写着"全库 8 张表没有任何变更历史表，上述信息不存在别处"。
那么追问一句：**这张表自己会被删吗？** 会。

### 实测证据

起第二个后端实例（`AUDIT_LOG_RETENTION_DAYS=91`、
`AUDIT_LOG_CLEANUP_INTERVAL_SECONDS=2`）：

```
INSERT 一行 created_at = now() - 100 days  →  存在
等 5 秒后再查                                →  0 行，被后台任务删掉
```

`AUDIT_LOG_RETENTION_DAYS` **默认 90 天**，清理每 3600 秒跑一轮，
`DELETE FROM audit_logs WHERE created_at < cutoff` 无条件执行。

| 缺口 | 实测依据 |
|---|---|
| 清理动作只进 `tracing::info!` | 进的是进程 stdout。不翻服务器日志就看不到，而"日志没了"恰恰是出事时最该被回答的问题 |
| 界面查不到保留策略 | `grep -rn retention src/controller/ frontend/src/` → 零命中 |
| 接口查不到保留策略 | 无任何端点暴露 `retention_days` |
| README 没写这四个变量 | 环境变量表里有 `RATE_LIMIT_*` / `LOGIN_*`，独缺 `AUDIT_LOG_*` |
| 后果从未被告知 | v0.13.0 之前 `result` 恒为空，删了无所谓；之后它是唯一副本，91 天后授权变更复盘能力归零 |

**管理员遇到的真实场景**：调查一起 3 个月前的授权变更，按日期一筛什么都没有，
而他无从判断这是"那天什么都没发生"还是"发生过但被清了"。
这两种解释导向完全相反的处置——这个歧义本身就是审计的失效。

### 修法一：清理动作自证（迁移 011）

新增 `audit_log_purges` 表，每轮**真的删了行**才记一行：
`cutoff_at` / `deleted_rows` / `ran_at` / `duration_ms` / `hit_batch_limit`。

不用 `audit_logs` 自己记：审计表记录自己的被删，会陷入
"删这行要不要连带记录、记录的那行算不算过期"的递归。
这张表与保留策略同寿命（策略只删 `audit_logs`），因此不会自己把自己删掉。

### 修法二：`hit_batch_limit` 区分"清干净了"与"撞上限收手"

删了 20 条、还剩 5 条过期数据时，只报"删了 20 条"会让人以为已经清完。
撞上限时必须说清"仍有更早的过期数据待下一轮清理"。

### 修法三：接口如实报告部署的真实策略

`GET /api/admin/audit-logs/retention` 返回保留天数、是否启用、清理间隔、
现存最早一条日志、最近一次清理。复用 `system:log:list` 权限码，
不为一个只读端点新授一个码。

### 修法四：界面按范围起点提示，而不是等空结果

范围 `[3 个月前, 今天]` 在只剩 1 个月日志时**仍会返回非空结果**，
但那 2 个月的数据一样是缺的。只在空结果时才提示的话，
用户会拿到一份"看起来查到了"的子集——这正是最难发现的漏查。
因此只要范围起点早于现存最早一条就提示。

### 修法五：文案在边界情况下不许说错

- 关闭清理时显示"日志不自动清理"，**不显示"保留 0 天"**——
  后者读起来像"日志随时都会被清光"，与实情完全相反
- 表为空时明说"当前没有任何日志"，不拿"最早一条"糊弄
- 拿不到策略信息时**不显示任何说明**：宁可不说，也不能显示猜出来的 90 天
- 非法时刻回落到破折号，不显示 `Invalid Date`

### 顺带修掉"靠别的用例先跑过"的前置依赖

五条直接 `INSERT audit_logs` 的用例此前只在**全量跑**时成立——
迁移由 `create_router` 触发，而它们自己不建 app，靠字母序靠前的用例
把库迁移过。单跑任何一条都会撞 `relation "audit_logs" does not exist`。
已加 `ensure_schema()` 守卫，并逐条在空库上单跑验证。

### 门禁

fmt / clippy 零警告 / 单测 **12**（新增前端文案单测）/ 集成 **+4** /
前端 typecheck·lint·build / e2e / 授权探针

缺陷注入三条，全部被承重用例抓到：

| 注入 | 结果 |
|---|---|
| `record_purge` 置空（清理不留痕） | ✅ 两条用例变红 |
| `hit_batch_limit` 恒报 false | ✅ 撞上限用例变红 |
| 前端文案隐去"撞上限"提示 | ✅ 对应单测变红 |

## [0.13.0] - 2026-10-03

主题：**审计要能回答"改了什么"。**

v0.10.0 修的是「界面不许说谎」，v0.11.0 让登录落审计，
v0.12.0 统一了错误响应形状。这一版接着补上审计的另一半：
它此前只能回答**「谁在什么时候调了哪个接口」**，
回答不了**「改了什么」**——而后者才是事后追溯真正要回答的问题。

### 实测证据：`params` / `result` 对所有写操作恒为空

```
 action                                    | params | result
 DELETE /api/admin/roles/<uuid>            |        |
 PUT    /api/admin/roles/<uuid>/menus      |        |
 POST   /api/admin/users                   |        |
```

`params` 只在有查询串时才有值（GET 筛选）；写操作的请求体**按设计不记录**
（口令、令牌入库即长期泄露面，这个取舍本身是对的）。于是：

1. **角色被删后名字永久丢失**——审计只剩一个 UUID，而 `roles` 行已删除，
    事后连"删的是什么角色"都答不出来
2. **授权授予无法复盘**——`PUT /roles/{id}/menus` 只知道"某角色的菜单被改了"，
    不知道授了/撤了哪些权限码。而这是整个系统里风险最高的操作
3. 全库 8 张表没有任何变更历史表，上述信息不存在别处

### 修法一：handler 显式声明摘要，中间件合并入库

新增 `AuditDetail`（`Arc<Mutex<Vec<String>>>` 挂在请求扩展上），
`middleware::audit_log` 在写库前把多条摘要合并进 `audit_logs.result`。

**刻意不自动记录请求体**：那会把 `password` / `old_password` / `new_password`
写进长期表，把"少记"换成"泄密"。改为由 handler 追加**它自己知道安全的那部分**
——不写就不入库，默认安全而不是默认危险。
handler 签名不因此变化，不写摘要的端点行为不变。

### 修法二：删掉的东西也要有名字

角色名只存在 `roles` 行里，行删掉就没了。新增 `utils::audit` 提供
`role_label` / `user_label` / `menu_label` / `dict_type_label` / `dict_item_label`，
删除类 handler 在 `DELETE` **之前**把名字读出来。

### 修法三：授权记差异，不记快照

`assign_role_menus` 是全量替换语义，只记提交上来的集合答不出"撤了哪些"——
而撤销恰恰是事后追溯最想知道的那一半。现在替换前后各取一次权限码快照求差，
审计里能查到"授予权限码 A、B；撤销权限码 C"。
两边都没变时记成"无变化"，重复提交同一份授权不是变更。

### 修法四：摘要必须能被断言，否则又是一个"以为记了"的字段

承重用例 `every_write_operation_leaves_an_answerable_change_summary`
**逐个走完 25 个写入口**，断言审计里能读到该读的事实（资源名、权限码差异、
口令重置的对象），并断言**口令一个字都不入库**。
配套 `every_documented_write_operation_is_covered_by_the_audit_test`
从 OpenAPI 派生写操作集合做清单自检：新增写端点却没纳入审计断言时当场变红
（这条自检在第一次运行时真的抓到了自己漏掉的一项）。

### 修法五：失败的写操作不得留下摘要

摘要的含义是"这次真的改了"。被拒绝的请求什么都没发生，
却记下"已授予/已删除"就是谎报——审计一旦开始说谎，比没有审计更危险。

### 附带：摘要存进库却读不到，等于没做

`result` 列此前**既不在界面表格里，也不在 Excel 导出里**。
审计日志是出事之后才有人看的东西，看不到就等于没记——
两处都已补上"变更摘要"列。

### 修三处"文档/注释说谎"

| 位置 | 原文声称 | 实际 |
|---|---|---|
| README 能力清单 ⚠️ 行 | 「非 admin 角色仍被 `require_role("admin")` 整体挡住」 | 该闸门连同 5 处调用**已在 v0.5.0 PR-3 整体删除**。把一个不存在的闸门说成现存边界，方向还是反的 |
| README 限流行 | 「固定窗口限流（IP 维度）」 | 实际是 **IP + 用户**双维度，且漏了 `RATE_LIMIT_USER_MAX` / `RATE_LIMIT_USER_WINDOW` 两个环境变量 |
| `frontend/src/utils/storage.ts` | 「简单的 **XOR** + Base64」 | 实现里没有 XOR，只有 `btoa(encodeURIComponent(...))` |

### 顺带查明：`code_a → code_b` 的改码路径在接口层走不通

不是缺陷，是两道各自正确的守卫合起来的效果：`update_menu` 要求调用者
已持有目标码，而目标码已存在时又撞唯一索引（迁移 `007`）返回 409。
实践中改码只能"清空 → 恢复/新建菜单"。
该摘要分支因此**无法被集成测试触达**（实测去掉 2xx 门禁后相关用例仍全绿），
改为抽成纯函数 `audit::permission_change` 用单测钉住。

### 界面与导出也要验，否则"记下了"仍可能读不到

新增 e2e 套件 `v013-audit-change-summary.mjs`（25 条断言）。
接口测试能证明库里那一列有内容，却证明不了界面表格和导出的 xlsx 读得到——
**一列加了但取错字段**（比如取 `params` 而不是 `result`）时，
上面所有测试都照样绿，只有把界面和文件打开才看得见。
套件因此验三件事：表头在、写操作的摘要格不是破折号、
xlsx 的 `sharedStrings` 里既有表头也有摘要内容。

### 顺带修掉两处"门禁自己在说谎"

| 位置 | 原文声称 | 实际 |
|---|---|---|
| `v010-ui-truth.mjs` | 断言"能翻到第二页" | 它能否成立取决于库里是否碰巧已有 10 个以上角色。干净库只有 `admin`/`user`，第二页压根不存在。该套件过去能过，只因跑它之前刚跑过集成测试、库被污染了——换干净库即红，而红的原因与被测界面无关。现改为套件自己补足到 `page_size + 1`，跑完逐个删并核对残留 |
| `every_query_dto_rejects_unknown_fields` | 逐个 DTO 校验 `deny_unknown_fields` | 只取结构体**紧邻上一行**，而 `RoleListParams` 的属性与结构体之间隔着 7 行注释和 `#[into_params(...)]`，于是误报。**这条用例在 v0.12.0 就已经是红的**——当时只统计了 `--ignored` 那组集成用例，非 ignored 组从未计入，"全绿"的说法当时并不成立。现改为向上遍历连续的 attribute/注释块 |

### 门禁

fmt ✅ / clippy 零警告 ✅ / 单测 **74** ✅ /
集成 **105**（`--ignored`）+ **8**（非 ignored）✅ /
前端 lint·typecheck·Vitest **93**·build ✅ /
e2e **6/6 套件** ✅ / 授权探针 **41/41** ✅

## [0.12.0] - 2026-10-03

主题：**入参不合法，任何端点都得长一个样。**

v0.10.0 修的是「界面不许说谎」，v0.11.0 补的是「出事之后能查」。
这一版收掉同族的另一半：项目对外承诺统一响应格式 `{ code, message, data }`，
前端拦截器也按 `message` 取文案（`frontend/src/api/index.ts`），
但**框架的默认入参路径绕过了这个承诺**。

### 修复一：请求体与路径参数的错误不再绕过 `AppError`

实测（起真实后端逐个打一遍，不是照文档推测）：

```
POST /api/admin/roles   body='{bad json'
  → 400  text/plain  "Failed to parse the request body as JSON: key must be a string at line 1 column 2"
PUT  /api/auth/password body='{bad json'
  → 400  application/json  {"code":400,"message":"错误的请求: 请求体不合法: Failed to parse..."}
```

同一个「入参不合法」，因为走的是不同提取器，响应形状就不同：

| 入口 | 迁移前 | 迁移后 |
|---|---|---|
| 19 处 `Json<T>` | 400/422 + `text/plain` | 400 + `application/json` |
| 17 处 `Path<T>` | 400 + `text/plain` | 400 + `application/json` |
| Content-Type 非 JSON | **415** + `text/plain` | 400 + `application/json` |

纯文本那一种在拦截器里取不到 `message`，用户只能看到一个空错误框——
这正是「承诺了格式却没兑现」的具体后果。

新增 `utils::api_extractor`（由 `json_extractor` 更名，装了两个提取器后旧名已不准）：

- `ApiJson<T>`：`JsonRejection` 四个变体**逐个翻译**，
  因此多传字段时消息里会指名是哪个字段不认
- `ApiPath<T>`：`PathRejection` 两个变体，保留 serde 原文，
  `/api/admin/users/not-a-uuid` 会指名参数而非只说「解析失败」
- 两者各自独立、**不合并**成泛型 `Api<T>`：合并只是把 `match` 分支藏得更深，
  而 `JsonRejection` 的四变体确实需要分别给文案
- 两个 `Rejection` 枚举都是 `#[non_exhaustive]`，兜底分支让 axum 小版本
  新增变体时不会直接把构建打挂

### 决定：415 并入 400

HTTP 语义上 415 更准确，但 v0.11.0 的改密端点**已经发布**并返回 400。
同一个逻辑错误因端点不同而返回不同状态码，正是本版要消灭的问题；
要改就得连同已发布行为一起改，那是破坏性变更，不该顺手做。
消息文本里已说明「必须带 Content-Type: application/json」，调用方仍能分辨。

### 承重测试：遍历 OpenAPI 全路由实测响应形状

36 处机械迁移最容易出的错就是漏一处，而**漏一处不会有任何编译错误**。
因此判据不写成「断言 36 处都改了」（那是自证），而是
`every_bad_input_returns_unified_error_envelope`：

- 探针表由 `docs::openapi_json()` **驱动**，不手写端点清单。
  漏改的那一处也在文档里，逃不掉；将来新增端点忘了迁移同样会进探针表
- 三类探针各对应 `map_rejection` 的不同分支：
  坏 JSON（`JsonSyntaxError`）、错 Content-Type（`MissingJsonContentType`）、
  非 UUID 路径（`FailedToDeserializePathParams`）
- 探针必须带**合法令牌**：鉴权中间件先于提取器跑，无令牌时会拿到 401——
  那也是 JSON 信封，会让断言「因为错误的原因而通过」，等于没测提取器
- 断言要求 `400` + `application/json` + 同时含 `code` 与 `message`，
  失败时打印出问题的端点与三种不合格原因
- 另有三条**探针表自检**（条数下限）：若 OpenAPI 结构变化导致一条都没解析出来，
  循环会空转、断言全绿——那正是本用例最怕的「因为没测到而通过」

共 54 条探针。缺陷注入验证：摘掉一处 `ApiJson` → 用例红（报出该端点的
`400 text/plain` 与 `415 text/plain`）；摘掉一处 `ApiPath` → 用例红
（报出该端点的 `400 text/plain`）；恢复后转绿。

### 顺带修复：角色列表的分页参数在文档里被标成了路径参数

实测 OpenAPI JSON 时发现的**真实文档缺陷**，与本版同族（文档说的和代码做的不一致）：

```
GET /api/admin/roles  →  "page"/"page_size" 的 in 是 "path"、required 是 true
```

但路径模板 `/api/admin/roles` 里根本没有 `{page}`——Swagger UI 会把它们
渲染成路径输入框，按 OpenAPI 规范校验也是无效文档。

根因：utoipa 的 `axum_extras` 本该从 handler 参数推断 `parameter_in`，
但 `list_roles` 显式接住拒绝，签名是 `Result<Query<RoleListParams>, QueryRejection>`
而不是裸 `Query<...>`，推断不出来 → 回落到 `ParameterIn::default()`，
而**那个默认值是 `Path`**（见 utoipa `openapi/path.rs` 的
`impl Default for ParameterIn`）。

显式钉 `#[into_params(parameter_in = Query)]`，不依赖这个默认值。

### 其他

- `src/utils/json_extractor.rs` → `src/utils/api_extractor.rs`（`git mv`，保留历史）
- 版本号 0.11.0 → 0.12.0（Cargo.toml / Cargo.lock / frontend/package.json）
- `middleware/permission.rs` 与 `controller/role.rs` 里两处提到「422」的注释已过时
  （本版起入参错误统一是 400），一并更正

### 未改动

- **成功响应体**一律不动。导出的 Excel/CSV、二进制响应天然不是 JSON，
  那不是「错误格式不统一」；承重测试只针对**入参**错误
- `docs/mod.rs` 里 `swagger_ui_handler` 的 `Path<String>` 保持 axum 原生：
  它是 SPA 静态资源回退路由，失败时该回 HTML/404，不是 JSON 信封

## [0.11.0] - 2026-10-03

主题：**补上安全追溯的基本盘，并给用户一条不依赖管理员的改密路径。**

前三版修的都是授权与数据一致性，这一版修的是**出事之后能不能查**。
登录与注册此前**完全不进审计**，失败只进 Redis 计数器——而计数器带 TTL 会过期。
事后能查到的只有"某段时间内失败次数偏高"，查不到"谁、从哪、在什么时候试过"。

### 修复一：登录与注册落审计

`/api/auth/login` 与 `/api/auth/register` 在 `public_routes` 里，
**没有挂 `audit_log_middleware`**，因此登录成功、登录失败、注册全部不进 `audit_logs`。

这不是"忘了挂"，而是**结构性不适用**：中间件依赖 `AuthenticatedUser` 扩展，
而登录请求本来就没有已认证用户，失败时更没有。
因此这三处审计必须在 service 里**显式写入**。

- `action` 用语义值 `AUTH_LOGIN_SUCCESS` / `AUTH_LOGIN_FAILURE` / `AUTH_REGISTER`，
  **不用**中间件那套 `{METHOD} {path}`。登录成功与失败的方法路径完全相同，
  只有 action 与身份有区分度——事后无法回答"有没有人在爆破"正是这一版的初衷
- 失败原因写进 `result` 列：账号不存在 / 口令不符 / 账号停用 / 已锁定
- **同步 `await` 写入**，写失败让整个请求失败。中间件的 `tokio::spawn`
  是为了不拖慢响应，而登录是低频且安全关键的路径：
  审计静默丢失等于没审计。代价是登录多一次 INSERT 往返
- `AuditLogRepository::record` 对 username/action/method/path/client_ip **截断**。
  登录失败时用户名是攻击者可控的超长串，不截断会把 INSERT 打挂——
  而"审计接口被一个超长用户名打挂"本身就是个新的拒绝服务面

### 新增：自助修改密码

此前改密只能靠管理员 `reset-password`，用户被重置才知道自己该改密码。

- `PUT /api/auth/password`，需验**旧口令**，且新口令不能与旧口令相同。
  只验复杂度是不够的：拿到一个劫持来的令牌就能把密码永久改掉
- 改密成功后吊销该用户**全部会话**（含本端），前端主动登出并提示
- 拦截放在后端 `auth_middleware`，不是前端跳转。
  只让前端跳改密页的话，令牌本身仍能调任何接口——
  那等于把权限校验交给界面，与 v0.10.0 关掉的"界面替后端承诺"是同一类错误
- `ChangePasswordRequest` 用 `deny_unknown_fields`：多传一个 `is_active` 或 `roles`
  直接 400 指名字段，而不是静默忽略后让调用方以为"改了"

### 新增：口令复杂度下限

- 长度下限 6 → 8，且要求至少 2 类字符
  （ASCII 大写 / ASCII 小写 / 数字 / 符号 / 非 ASCII 字母，五类）
- 取 2 类而非 3 类，是为了不误伤 `admin123`——它是 README 与 e2e 的默认账号
- **策略只在"设置口令时生效，登录时不校验**。
  否则把门槛一抬，存量弱口令用户当场被锁在门外
- 前后端各存一份实现，因此共用一份判定样例 `PASSWORD_POLICY_CASES`：
  后端有一条集成测试读前端源码、用 Rust 的 `validate_password` 跑同一批口令并比对结论。
  规则在任一侧被改动而另一侧没跟上，那条测试立刻变红

### 新增：首次登录强制改密（不叠加第二次强制登出）

管理员新建/重置的用户须先改密。做法是**受限令牌**，而不是"登录即踢下线"——
`iat_ms` 升级已经让存量令牌作废过一次，再来一次同类冲击是没必要的。

- `users.must_change_password` 默认 `FALSE`，**存量用户完全不受影响**。
  只有管理员新建/重置的用户才置 true
- 登录时若为真，JWT 带 `pwd_stale` claim，
  `auth_middleware` 只放行改密 / 登出 / `/me`，其余一律 403「请先修改初始密码」
- 用户改完密拿到正常令牌，全程不丢工作、不必重新输密码
- 前端把 `/profile` **静态注册**在 `MainLayout` children 里，不走后端菜单。
  菜单是按角色授权的，塞进去会让"角色没勾这一项"的用户连改密入口都没有——
  而"改不了密码"正是管理员重置口令想解决的问题

### 顺带修掉的两处

- **改密后前端不再调 `/auth/logout`**：改密成功时后端已吊销全部会话，
  再发一次登出请求必然 401。一次注定失败的往返，
  外加控制台里一条 `Failed to load resource`，把真正值得看的错误淹掉
- **受限用户进个人中心不再刷一屏「权限不足」**：路由守卫明知菜单必然 403
  仍去加载菜单与权限码，一次落地弹 4 个错误提示。现在在守卫里就 return

### 遗留（已记录，未处理）

`Json<T>` 解析失败仍走 axum 默认路径返回 **422 + text/plain**，
绕过项目承诺的统一 `{code,message,data}` 格式（与既有 `QueryRejection` → 400 不一致）。
本版新增 `ApiJson<T>` 只用在改密端点，其余 18 处待后续统一。

## [0.10.0] - 2026-10-02

主题：**停止说谎——把"界面提供了控件、后端却没有对应能力、且失败时无声"这一族缺口关掉。**

与前三版同源（"授权的两面只装了一面"的 UI 版本），但危害不同：
安全洞是**放行了不该放行的**，而假接口是**让人以为能力存在而其实没有**。

**本版真正的重点不是把三个筛选做出来，是让"未知参数"不再静默丢弃。**
只要 `serde` 继续默认忽略未知字段，将来新增任何筛选条件都会再次悄无声息失效，
前两项修完也会复发。

### 修复一：用户列表的搜索框真的能搜到人

搜索框把 `keyword` 发到了后端，后端也**收到了**——然后安静地忽略了它。
因为 `UserListParams` 没有这个字段，而 `serde` 默认丢弃未知字段。
界面表现为"搜索没反应"，不报错、不留痕。

- `keyword` 同时匹配用户名与邮箱
- LIKE 模式先过 `escape_like_pattern`：`%` 与 `_` 是用户输入的**字面量**，
  不是通配符。此前搜 `%` 会返回全表，搜 `_` 会匹配任意单字符
- 空串等同不过滤（搜索框清空后前端会发空串，当成关键字就会筛出零条）

### 修复二：审计日志的筛选条件真的生效

`GET /api/admin/audit-logs` 此前**一个筛选参数都不支持**，
而界面摆着"操作""用户名"两个输入框。

- `username` / `action` 模糊匹配，`status_code` 精确匹配，
  另有 `start_time` / `end_time` 时间范围
- 筛选用 `QueryBuilder` **动态拼接**，不用 `($1::text IS NULL OR ...)` 那套恒真写法：
  后者虽然省事，但 Postgres 看见的是"对每列都可能为真"的 OR，
  优化器无法把它化成索引扫描——日志表越大越慢。
  动态拼接下每个条件缺失时**根本不进 SQL**，索引照常可用

### 修复三：导出不再静默截断

`LIMIT 10000` 本身不是问题（日志表是唯一会无限增长的表，没有上限迟早 OOM），
**静默截断才是**：用户以为导出了全量，实际只有最新一万条，且没有任何提示。

- 导出支持与列表同一套筛选条件。此前界面上"筛完再导出"，导出的是**全量**
- 多取 1 行判断是否触顶，响应头回传
  `x-export-row-count` / `x-export-truncated` / `x-export-max-rows`
- 前端读 `x-export-truncated`，为真时明确告知"已导出 N 条，达到上限，数据不完整"

**顺带修掉一个会吞掉上面那条提示的 bug**：前端的 GET 缓存命中时返回的是
伪造 response，`headers` 里只有 `{'x-cache': 'HIT'}`。若导出走了缓存，
`x-export-truncated` 读不到，界面会在数据被截断时照样报"导出成功"——
又变回无声失败。现二进制响应不参与缓存。

### 修复四：未知参数不再静默丢弃

本版的核心决定。所有 query DTO 一律 `deny_unknown_fields`：
多传一个拼错的参数，现在直接 **400 并指名那个字段**。

代价与前提：

- **不兼容**：原先依赖"多传参数被忽略"的客户端会开始报错。这是有意的——
  静默忽略正是这一族 bug 的成因
- **与 `serde(flatten)` 不兼容**（serde 明确不支持二者共用），
  所以查询结构体必须**显式列全字段**，不能靠 flatten 复用 `PaginationParams`

覆盖：`UserListParams`、`AuditLogQuery`、`RoleListParams`、`MenuQuery`、`DictItemQuery`。
后两个此前**连这个属性都没有**，且 dict 的 `list_items` 还在走绕过统一响应格式的
旧写法（`Query` 提取器解析失败时 axum 默认回 `text/plain` 400），一并修正。

### 修复五：`BaseUpload` 从演示页摘掉

组件示例页摆着一个上传控件，`:action` 写死 `'/api/upload'`——
**这个端点在路由表里根本不存在**。控件看得见、点得动，得到的只是一个 404。

组件本身保留在 `components/common/BaseUpload.vue`。
文件上传要牵出对象存储、病毒扫描、内容类型校验与配额限制，是独立议题，
不该为一个演示页草率接上。等后端真有了上传端点再放回来。

### 变更：角色列表分页（**破坏性**）

`GET /api/admin/roles` 的返回值从 `Vec<RoleItem>` 改为分页对象：

```diff
- { "code": 200, "data": [ { "id": …, "name": … } ] }
+ { "code": 200, "data": { "items": [...], "total": 34, "page": 1, "page_size": 10, "total_pages": 4 } }
```

角色数会随自定义角色增长，不分页就迟早把整张表塞进内存并渲染成一屏。

- 排序 `ORDER BY r.created_at ASC, r.id ASC`：`created_at` 会重复，
  只用它排序会让翻页出现重复/漏行，`r.id` 是并列时的 tiebreaker

### 回归资产：契约测试与界面自查套件

这一族问题端到端测试抓不到——请求确实成功，只是筛不出东西。
只能靠把两边的字段名拿来对照。

- `frontend_query_params_match_backend_dto_fields`：5 组前后端字段对照，
  **双向**报告（前端发了后端不认识的字段 / 后端有、前端从不发的字段）
- `every_query_dto_rejects_unknown_fields`：扫描 `src/controller/*.rs` 里所有
  `*Query` / `*Params` 结构体，要求带 `deny_unknown_fields`。
  **不靠人列清单**，新增 DTO 自动纳入检查
- `e2e/suites/v010-ui-truth.mjs`（13 条断言）：证明界面把参数**发出去了**——
  控件摆着但没接上事件，是同一族问题里最常见的一种

## [0.9.0] - 2026-10-02

主题：**授权边界的最后一面 + 把探针从一次性脚本变成仓库里的工具。**

本版合并原先排成 v0.9.0 / v0.10.0 / v0.11.0 的三项工作。三者性质不同
（安全修复 / 测试资产 / 工具），但落在同一条"把偶然变常规"的主线上，
且后两项恰好是验证第一项的基础设施。

### 修复一：`assign_user_role` 的两处不对称

同一个端点上，此前只查了"授予什么"，没查"授予给谁"、也没管"生效没有"。

- **不检查目标用户**。只持 `system:user:update` 的角色能给一个纯 admin 账号
  追加角色（实测 200，角色真的变了），而 `update_user` / `delete_user` /
  `batch_delete_users` 三处**都**查目标。现与它们对齐
- **追加后不撤销存量会话**。新授的权限码要等目标用户自己重新登录才生效
 （实测：追加后原令牌仍 403，重新登录才 200）。现与 `update_user` 同一套处理
- **目标用户不存在返回 404 而不是 500**。此前直接写 `user_roles`，
 外键违例冒成「服务器内部错误」——与 v0.8.0 修的"声明已占用的权限码冒成 500"
 同源：入参错误被当成服务端故障，既污染错误监控，调用方也看不懂
- **重复追加同一角色不再踢掉目标用户**。该写入本是 `ON CONFLICT DO NOTHING`
 的幂等操作，照样吊销会话等于把一次无操作变成一次强制登出

**不阻断"给自己追加弱角色"**：调用者天然覆盖自己的全部权限码，
这条守卫对自己恒真——合法的自我降级路径不会被误伤。

### 修复二：会话吊销的毫秒精度（`iat_ms`）

`POST /roles` 分配角色后不生效的根因不只是漏调吊销——**秒级精度也分不开**。

JWT 标准的 `iat` 只有秒级精度。若吊销水位也只存到秒，
"令牌在吊销之前签发"与"之后签发"会落在同一秒内无法区分，
于是任何秒级方案都只能二选一：**放过旧令牌（漏吊销）**或**误伤新登录**。
两个都是错的，且都是可复现的（登录后立刻改角色，旧令牌仍可用）。

现 `Claims` 新增 `iat_ms`（毫秒签发时间），与毫秒级吊销水位配套。
**不能拿它替代 `iat`**：`exp` 的校验由 jsonwebtoken 按标准秒级 `iat`/`exp` 完成。

`serde(default)` 让升级前签发的旧令牌仍能解析（此时为 0），
0 一定小于任何吊销时间点——方向是**失效**而非放行，即升级后首次吊销会把
存量令牌一并作废。这是安全的一侧。

### 修复三：删除角色也要过授权天花板

探针工具第一次运行就抓到的洞，与 v0.8.0 的 `delete_menu` **同一形状**：
授权的两面只装了一面。

只持 `system:role:delete` 的操作员能删掉一个承载 `system:log:list` 的角色
（实测 200，角色真被删），而那个码他自己并不持有。
删角色 = 把这个角色承载的权限码从所有人身上撤走，
与 `PUT /roles/{id}/menus` 是同一件事的两面——那条路 v0.7.0 装了这道天花板，
删除这条路当时没管。

守卫放在**内置角色检查之后**：内置角色永不可删是与权限无关的固有事实，
若先报"缺少权限：X"，操作员会误以为拿到 X 就能删内置角色——
那是在把人往错误方向引。

不像 `delete_menu` 那样要判断"是否有人依赖"：角色只要存在，
它携带的码就都在生效，删掉必然改变每个人的权限。

### ⚠️ 行为变化：删带自定义码的角色必须先撤授权

现在删除一个承载"你未持有的码"的角色会被拒（403）。

副作用是：admin 自己造的码分发给别的角色后，**admin 反而删不掉那个角色**了。
这与 v0.8.0 的菜单删除同一性质（v0.8.0 的 e2e 里 admin 同样被拒过），
是既有设计的延续而非新引入的意外。恢复路径：先撤销授权，角色变成
"不携带任何码"后删除即放行。

### 新增：授权探针工具化（`e2e/probe-write-guards.mjs`）

v0.7.0 / v0.8.0 / v0.9.0 的洞都是探针跑出来的，不是读代码看出来的。
但探针一直是一次性手写的，47 个 handler 逐个手写不现实，
于是每版都靠"记得检查"——而"记得"正是会漏的那一环。

**写入口清单不靠人列，从 OpenAPI 自动发现**。新加一个 `POST /api/admin/xxx`
自动进入探针视野；**没登记探测方式就直接报"未覆盖"**，
逼着人当场决定，而不是留到下一版才发现漏了。当前发现 24 个写入口，
全部有登记或明确豁免。

判定标准是一个可证伪的命题：只持入口所需**最小权限码**的操作员，
对"权限高于自己"的目标发起写操作 → 必须被拒，且数据不得有任何变化。
（必须用最小权限码：若给操作员发 admin，被拒只可能来自目标下界检查，
就测不出"入口权限码本身够不够"的真实边界。）

每个条目**三条断言，缺一不可**：弱操作员被拒 / 数据侧未变 /
**admin 做同一件事仍然成功**。第三条对破坏性入口尤其重要——
只测拒绝侧的话，"把入口整个禁掉"也能全绿。

### 新增：浏览器回归脚本进仓（`e2e/`）

`regress.mjs` + `delguard.mjs` 等约 22KB 原先在 `/tmp/axum-e2e/`，
配置全是硬编码路径，**仓库内无 e2e 目录，会丢**。

这层补的是单元测试和 `tests/api_integration.rs` 天然看不见的东西：
按钮看得见但一提交就 403（前后端权限码对不上）、
路由能进但页面空白、跨身份场景（建号 → 弱操作员越权 → 目标用户验会话）。

零依赖：Node 22 内置 `WebSocket` 与 `fetch` 直连 CDP，不需要 `npm install`，
拉进 ~300MB 的 Playwright 不划算。代价是要自己处理 target/session 分离。
配置全改为环境变量；harness 自己拉起 headless Chrome，
若 9222 上已有浏览器就复用，且**只关自己拉起来的那个**。

### 测试：集成测试 65 → 73（新增 8 条）

- `appending_a_role_to_a_stronger_account_is_denied`
- `appending_a_weaker_role_to_yourself_is_still_allowed`
- `appending_a_role_takes_effect_without_waiting_for_a_relogin`
- `re_applying_the_same_role_does_not_kill_the_target_session`
- `appending_a_role_to_an_unknown_user_is_not_found`
- `deleting_a_role_that_carries_permissions_you_lack_is_denied`
- `deleting_a_role_whose_permissions_you_cover_is_allowed`
- `deleting_a_role_that_carries_no_permission_is_allowed`

**缺陷注入验证**（都真实失败后还原）：移除 `iat_ms` 改回秒级比对 → e2e 套件
精准变红且失败形态是 **403 而非 401**，即旧令牌被**放过**（漏吊销方向）；
移除 `delete_role` 天花板 → 探针 39/41，两条精准变红。
两侧看到的不是同一个 bug 侧面（集成侧暴露"误伤新登录"，e2e 侧暴露"漏吊销"），
都得留着。

e2e 首跑还抓到一个**假绿**：运行中的二进制与源文件 mtime 撞在同一分钟，
旧二进制只编进了更早改的那一半，于是所有令牌一律 401——
看起来像"毫秒化有 bug"，实际是部署物没更新。
教训：mtime 撞同一分钟不可信，一次通过的探针只能证明它验的那一处。

### 门禁

| 项 | 结果 |
| --- | --- |
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | ✅ |
| 单元测试 | ✅ 55 |
| 集成测试 | ✅ 73 |
| 真实 Chrome 回归 | ✅ 3/3 套件（13 + 14 + 24 条断言） |
| 授权探针 | ✅ 41/41 |
| 前端 lint/typecheck/test/build | ✅ |

## [0.8.0] - 2026-10-02

主题：**菜单删除的授权下界——补上 v0.7.0 只修了一半的那个洞。**

v0.7.0 给 `update_menu` 加了守卫：清空一个**别的角色正依赖**的权限码，
必须持有该码，否则等于绕过 `system:menu:grant` 完成一次跨角色撤权。

但**同一条路还有另一个入口**：`delete_menu`。删掉一个承载权限码的按钮，
效果与清空完全等价（码从所有依赖它的角色身上消失），却**没有任何检查**。

### 修复

- **删除承载权限码的菜单现在要求持有该码**。守卫覆盖**整棵子树**而不只是
 目标节点：`menus.parent_id` 声明了 `ON DELETE CASCADE`，删父目录会连带删掉
 子树里承载码的按钮——只查目标节点会漏掉这条路，而它与直接删那个按钮
 效果完全相同
- **只对"已被授予至少一个角色"的码设限**。没人依赖的码删掉不改变任何人的权限，
 照旧放行；否则"整理菜单结构"这类无害操作会全线报错
- **声明一个已被占用的权限码返回 409 而不是 500**。迁移 `007` 的部分唯一索引
 早就挡住了重复声明，但报出来的是 500「服务器内部错误」——入参错误被当成
 服务端故障，既污染错误监控，管理员也看不懂发生了什么
- **非法的菜单类型返回 400 而不是 500**（`menus_type_check`）。
 与上面那条同源：`type` 只允许 `menu` / `button` / `directory`，
 传别的值原本也冒成 500。两条约束现在统一由一个映射函数翻译成业务错误，
 覆盖 `create` 与 `update` 两条写路径

### 实测：修复前的撤权链

用一个**只持** `system:menu:list` + `system:menu:delete` 的角色
（既不持那个码，也没有 `system:menu:grant`）：

```
carrier 持有的码 before = ["probe:revoke:02f64"]
deleter 的删除请求        = 200 OK
carrier 持有的码 after  = []
```

自提权路径是闭合的（`ensure_can_grant_roles` 要求调用者已覆盖目标角色的码，
把自己塞进那个角色会被拦下），所以这是**跨角色撤权**而非自提权——
但撤权本身已经足够严重。

### 守卫不会把菜单永久锁死

admin 造出一个码后并不持有它，因此 admin 也**不能直接**删除承载该码的菜单。
恢复路径是产品设计的一部分：先撤销该菜单的授权（需 `system:menu:grant`），
菜单变成"没人依赖"后删除即放行。

既有三条 v0.7.0 用例正是在清理步骤卡在这里（403）——**那是正确行为而不是缺陷**，
它们的清理方式已相应改为走这条恢复路径，并顺带把"没有锁死"变成了断言。

### 测试：集成测试 63 → 68（新增 5 条）

- `deleting_a_granted_button_others_rely_on_requires_holding_it`
- `deleting_a_directory_with_a_granted_button_below_is_denied`
- `deleting_a_button_no_role_relies_on_is_allowed`
- `declaring_an_already_used_permission_code_is_a_conflict`
- `an_invalid_menu_type_is_a_bad_request`

**缺陷注入验证**（都真实失败后还原）：

| 注入 | 结果 |
|---|---|
| 移除 delete 守卫 | 两条删除用例同时变红 |
| 子树查询去掉递归（只看目标节点） | 目录级联那条变红，直接删按钮那条**仍绿** |
| 移除占用预查 | 冲突用例**仍绿** |
| 预查与索引映射**都**移除 | 冲突用例变红 |
| 移除 `menus_type_check` 分支 | 类型用例变红 |

第二条是鉴别性结果：证明递归查询真正承重，且两条用例覆盖的是**不同的**攻击路径。

第三条暴露一个诚实缺口：占用预查与唯一索引映射两层都在报冲突，
测试只能区分"两层都没了"。即**预查本身没有被独立覆盖**——
它负责的是那条可操作的消息文案，正确性由索引映射那层兜底。

### 真实浏览器回归 13/13

集成测试证明不了"守卫没有误伤正常管理流程"，所以用**真实 Google Chrome**
（CDP 驱动 `--headless=new`）跑了一遍：真实登录 admin → 新建并删除无码页面菜单
（放行，证明日常整理菜单没被误伤）→ 新建带码按钮并授予另一个角色 →
删除被拒 403 且码没被剥掉 → 撤销授权后删除放行（恢复路径真实可用）
→ 菜单管理页正常渲染、无 console error、除预期 403 外无 4xx/5xx。

界面上的报错文案是：`缺少权限：删除承载权限码「xxx」的菜单需要「xxx」，
而你未持有该权限码`——管理员能直接看懂缺的是哪个码。

### 原计划的前提被实测推翻

本项原计划是"给 `create_menu` 补授权下界，只允许声明当前没被占用的码"。
实测发现那个洞**早就被迁移 `007` 的唯一索引堵上了**，只是报成 500；
真正没堵的是 `delete_menu`。因此本版改修 delete，预算是计划阶段记错的。

## [0.7.0] - 2026-10-02

主题：**权限码清空后的恢复路径——把"自己删掉的权限码找回来"变成一个正常操作。**

`update_menu` 的授权守卫只管"写入"，不管"清空"：守卫条件里的
`.filter(|p| !p.is_empty())` 让清空整个绕过了检查，于是有两条后果。

### 修复

- **清空权限码是条死路**：码被清空后全系统就没有任何角色再持有它，
 而守卫要求"改写权限码必须持有目标码"——于是**写回去会被自己的守卫拦死**。
 唯一出路是新建一个孤儿按钮，原按钮的 `role_menus` 授权还在、
 却不再对应任何码，管理员也看不出原来那个码是什么。
 现新增端点 `POST /api/admin/menus/:id/restore-permission`：
 清空时把旧值与清空者记进 `menus.prev_permission` /
 `prev_permission_cleared_by`（迁移 `009`），清空者可一键还原
- **清空别人的权限码可绕过 `system:menu:grant`**：持 `system:menu:update`
 的角色可以把**别的角色**已持有按钮的码清掉，完成一次跨角色撤权。
 现要求：清空的按钮**已被授予至少一个角色**时，必须持有该码；
 没授予任何角色的照旧放行（那种清空不改变任何人的权限）

### 为什么恢复不构成提权

清空一个**已授权**按钮的码要求持有该码，因此"能清空"蕴含"清空前持有"。
恢复只是把状态还原到清空之前，**净零**。按钮本就未授予任何角色时，
清空放行、恢复也只是给"无人"一个码，同样净零。
而"把菜单授予别的角色"仍要过 `system:menu:grant` 与既有判定，
故本版不构成新的越权原语。恢复凭据**一次性作废**，不能反复清空/恢复。

### 新增

- 菜单树节点返回 `restorable_permission`，前端据此显示
 "码已清空，可恢复 X"标签与恢复按钮。
 **刻意不暴露** `prev_permission_cleared_by`：界面只需知道能不能恢复，
 "是不是你清的"由服务端按 user id 判定

### 变更

- 清空权限码时统一落成 `NULL`（此前空串与 NULL 混用）。
 授权查询本来就靠 `permission <> ''` 把两者等价过滤，
 统一后"这个按钮当前有没有码"才是一个确定的事实
- `menus` 到 `Menu` 的列清单收口成 `MENU_COLUMNS` 常量。
 `sqlx::FromRow` 要求结果集包含结构体每一个字段，本仓库原有 4 处
 各自手写列名，加列时差点只改一半

### 测试

- 集成测试 51 → **56**（新增 5 条）
- **缺陷注入验证**：移除清空守卫 → 跨角色撤权那条测试实得 200（期望 403）；
 移除恢复的归属校验 → 非清空者接管那条测试实得 200（期望 400）
- 空库重建（0 张表）后跑全量，`max_migration=9`，
 种子核验 42 菜单 / 28 权限码 / 无残留

### 补齐前端入口（计划第 3 项）

后端有、前端无入口的权限码有两个，本版各补一处入口：

- `system:monitor:export` → 系统监控页页头「导出 Excel」按钮
 （`GET /api/admin/monitor/system/export`，blob 下载）
- `system:test:access` → 后端能力示例页新增第 4 张卡「能力探测」，
 一键打 `GET /api/admin/test`。按钮可见却报 403，
 就说明前端拿到的码与后端认的码对不上——这是最直接的排查手段

契约测试 `permissionCodes.spec.ts` 的 `BACKEND_ONLY_CODES` 豁免清单
因此**清空**：它本来就是用来登记「后端有码、前端没入口」的，
清空意味着后端 28 个权限码在前端全部有入口，此后双向全覆盖，
将来新增任何权限码都会让测试失败，逼迫显式判断是否需要入口

### 修复：后端能力示例页的分页表格首屏恒为空

`fetchTest` 只挂在分页的 `@update:page` 上，从未在 `onMounted` 触发——
进页面表格显示「无数据」，要点一次页码才出数据。
已补 `onMounted(fetchTest)`

### 测试：真实浏览器回归 13/13

v0.5.0 起挂着的债（本版动了前端，必须补上）：用**真实 Google Chrome**
（CDP 驱动，`--headless=new`）跑完整链路——真实登录 admin →
监控页点导出 → 文件落盘且为合法 xlsx（`PK` 魔数 + `xl/worksheets/sheet1.xml`）
→ 能力探测返回「管理员访问成功！」→ 全程无 4xx/5xx、无控制台错误。
上述分页表格缺陷即是靠人工核对截图发现的，已在回归脚本里补成硬断言

### 运维债：指标路径归一化 + 聚合到 Redis（计划第 4 项）

这是计划第 4 项的上半部分。三件事：

- **接口耗时跨副本聚合**。指标原先是进程内 `HashMap`，有三个在多副本下
 变错的问题：重启即丢、多副本各看各的（监控页上的 QPS 只是本副本份额）、
  且 `reset` 只清本进程导致"重置"后数字立刻涨回来。
 现改为「本地增量缓冲 + 定时 flush 到 Redis」——请求路径上只做一次内存合并
 （无网络往返），后台任务把增量 `HINCRBY` 进共享 Redis。
 max/min 用 Lua 原子更新以免跨副本竞态；flush 失败保留增量下轮重试；
 优雅关闭前做最后一次 flush
- **指标路径改用路由模板**（`/api/admin/users/{id}` 而非含真实 UUID 的路径）。
 旧实现按原始路径建键，每个资源 ID 是一条独立记录：基数无界，
 且每条 `call_count` 恒为 1——监控页看不出任何接口的真实 QPS。
 这是上一条的前置：若不归一化就落 Redis，等于每个 UUID 一个 Redis 键
- **顺带修 `min` 的 0 值哨兵**。原先拿 `0` 当"无样本"，
 但亚毫秒请求的耗时真的就是 0，会被后续更大的值覆盖。改用 `Option<u64>`

### 运维债：审计日志保留策略（计划第 4 项）

`audit_logs` 每来一个已认证请求插一行且无清理。新增后台保留任务：

- 按 `AUDIT_LOG_RETENTION_DAYS`（默认 90 天，0 = 关闭）定期删除过期行
- **分批**删除（`AUDIT_LOG_CLEANUP_BATCH_SIZE`，默认 10000）：
 一次性删大量行会长时间持锁并撑爆 WAL，分批把锁持有时间切碎
- **删除留痕**：审计数据被静默删除不可接受，每次删除打 `info` 日志
- 保留策略与指标 flush 都由 `main` 启动后台任务、优雅关闭时收尾，
 不放进 `create_router`（否则集成测试反复建应用会留下任务打共享测试库）

### 测试：计划第 4 项

- 单元测试 49 → **55**（新增 6 条，覆盖合并算术与 0 值 min 语义）
- 集成测试 56 → **63**（新增 7 条）：路径模板归并、跨副本聚合、
 跨副本重置、未 flush 增量可见、部分已落库端点仍为一行、审计保留只删过期、
 分批且受上限约束
- **缺陷注入验证**：路径改回原始路径 → 归并测试实得 2 行 `call_count=1`；
 `reset` 改回只清本地 → 跨副本重置测试实得残留数据
- 注入过程中还暴露了实现里两个真 bug，均已修并补测试：
 拆 Redis 键忘剥 `metrics:ep:` 前缀导致 method 变成 `metrics:ep:GET`；
 Redis 键与本地缓冲键格式不一致导致同一端点裂成两行（重复计数）
- 真实 Chrome 回归：接口监控页按模板聚合、`method` 干净

## [0.6.0] - 2026-10-02

主题：**多角色用户——修掉"保存一次，其余角色静默消失"。**

v0.5.0 把授权闭环接到了界面上，但接的过程中留下了最后一处
**静默数据丢失**：数据模型（`user_roles` 表、`UserInfo.roles`）
一直是多角色的，而用户表单提交的是单数字段 `role`，服务端对它做的是
**整体替换**。于是任何"给一个已有角色的人再添一个角色"的操作，
点保存后其余角色全部消失，接口还返回 200。

这是一个矛盾组合：**数据库能存多角色，读接口返回多角色，
唯独唯一的写入口只接受一个角色**。本版把写路径补齐，并让界面
如实呈现多角色。

### 修复

- **保存用户会静默删除其余角色**（本版核心缺陷）：
 `UserManageRequest.role` 是单数，而 `replace_user_roles` 是整体替换语义，
 前端又只回填 `roles[0]`。多角色用户一打开编辑框、一保存，
 除第一个角色外的全部授权被无声删除，且响应是成功。
 请求体新增权威字段 `roles: string[]`，`role` 保留为 deprecated 别名
 （两者都给时以 `roles` 为准），既有客户端零改动
- **用户列表把多角色显示成单角色**：列表"角色"列只渲染 `roles[0]`，
 持多个角色的用户看起来与单角色用户无异。现渲染为多标签
- **部分角色写入的中间态**：角色存在性校验改为**先全校验再写**，
 任一角色不存在即整体拒绝，不留"部分角色已生效"的半成品
- **重复角色产生重复行**：表单多选可能重复提交，
 现去重并保持提交顺序（`user_roles` 的 `ON CONFLICT DO NOTHING` 此前会静默吞掉重复项）

### 变更

- **零角色用户被接口拒绝**：`roles` 为空时返回 400。
 没有角色的用户登录后拿不到任何权限码、侧栏也是空的，
 属于"建得出、没人能用"的死数据，与 v0.5.0 修掉的半成品用户同类。
 如需先建号后授角色，走"创建 → 再分配"的正常流程
- 授权下界判定（`ensure_can_grant_roles`）、最后一个管理员保护
 （`ensure_not_last_admin`）改为接受完整角色集合而非单角色。
 两者本就按切片实现，**判定语义不变**：
 仍要求"你持有的权限码 ⊇ 目标角色集合的并集"，多角色不会放宽授权

### 破坏性变更

- `UserManageRequest.role` 由 `String` 改为 `Option<String>` 并标记
 `#[deprecated]`。**对外行为向后兼容**：仍只发 `role` 的客户端行为与
 v0.5.0 完全一致（旧集成测试 `the_legacy_single_role_field_still_assigns_one_role`
 未作任何改动即通过）。仅在 Rust 侧直接构造该 struct 的代码需要调整

### 测试

- 集成测试 44 → **51**（新增 7 条）：多角色创建/更新往返、空角色列表被拒、
 单个非法角色整体拒绝、重复角色折叠、旧单数字段兼容、
 多角色同时授予仍触发超集规则
- 单元测试 **49**，前端单测 81 → **84**
- 缺陷注入验证：把 `requested_roles` 退化为 `vec![roles.first()...]`
 后，`creating_a_user_with_several_roles_assigns_all_of_them`
 实际失败（`left: ["role_a_…"] right: ["role_a_…", "role_b_…"]`），
 确认新测试真的能捕获该缺陷
- 在**完全空库**（0 张表）上重跑全量集成测试，验证空库迁移 + 种子数据引导

## [0.5.0] - 2026-10-02

主题：**角色与授权闭环——授权写路径不再静默失效，把角色管理能力接到界面上，
并撤掉粗粒度角色闸门，让授权彻底由权限码决定。**

v0.4.0 把权限码变成了强制鉴权，却没有任何一条正常路径能**配置**它：
角色管理页只有一张只读表格，授权只能改数据库。本版接上 UI，
但动手前的审查发现授权写路径本身是坏的，先修后接。

本版分三个 PR：**PR-1** 修授权写路径并把角色/菜单授权接到界面；
**PR-2** 拆掉"可分配角色"白名单，让自定义角色真正能被分配出去——
否则 PR-1 建出来的自定义角色是个建得出、却没人能用的空壳；
**PR-3** 撤掉 `require_role("admin")` 粗粒度角色闸门，
并补上撤掉它之后必须有的授权下界。

### 修复

- **创建用户会留下半成品用户**：`create_user` 先建用户行、再调 `replace_user_roles`
 校验角色，两者不在同一事务。角色非法时接口报错，但**用户行已经落库**，
 留下一个"没有任何角色"的用户，而前端提示的是失败。现把角色存在性校验
 提到写用户之前
- **重命名内置角色等同删除它**（PR-1 修了 `delete_role`，漏了 `update_role`）：
 `ADMIN_ROLE = "admin"` 是"最后一名管理员"保护与权限码种子的查找依据，
 改名后这些依据全部落空，系统会变成没人是管理员。现拒绝改名内置角色，
 也拒绝把自定义角色改名成内置角色名
- **`update_role` 返回编造的数据**：`created_at` 恒为 `now()`、`user_count` 恒为 0
 ——改一个 50 人角色也会回一个"0 人、刚创建"的角色。改为回读真实行
- **角色撞名返回 500**：唯一约束违例被当成内部错误。归一化后撞名会变成常见操作
 （先建 `Auditor` 再建 `auditor`），现返回 409
- **编辑用户会静默改掉自定义角色**（接 UI 时发现）：前端用 `UserInfo.role` 回填表单，
 而该字段是只有 admin/user 两值的**展示枚举**，非 admin 一律塌缩成 `user`。
 自定义角色的用户一打开编辑框就变成"普通用户"，一保存角色就被改掉。
 改用真实角色集合 `roles` 回填与展示
- **撤销权限静默失效**：`assign_role_menus` 对 DELETE 与 INSERT 都用 `.ok()` 吞掉错误却
  仍提交事务，于是「取消勾选 → 保存」可能什么都不发生，而 UI 报「权限分配成功」。
  现错误上抛；INSERT 改为单条 `INSERT...SELECT unnest`，**任一 ID 非法即整体回滚**
  （此前传「合法 + 非法」混合 ID 会静默部分授权并返回成功）
- **`GET /api/admin/menus?role_id=` 对部分授权的角色返回空树**：`build_tree` 只把
  `parent_id IS NULL` 当根，父节点不在过滤结果里的节点被**静默丢弃**。
  admin 因被种子授满全部 42 个菜单而恰好看不出问题。对未授满祖先的角色，
  授权弹窗会显示「该角色没有任何权限」，管理员一保存就按全量覆盖把授权清空。
  改为森林语义：父节点不在集合内即视为根
- **`delete_role` 无守卫且非事务**：原先可删掉 admin 角色。角色种子仅在 `roles` 表为空时
  写入，删掉不会重建，系统将**永久**失去该角色。现单事务 + `FOR UPDATE`，
  拒绝删除内置角色，并报出仍占用该角色的用户数
- **`delete_menu` 手写递归删除多余且吞错**：`menus.parent_id` 与 `role_menus.menu_id`
  均已声明 `ON DELETE CASCADE`，数据库已原子级联。改为单条 DELETE 并按
  `rows_affected` 返回 404

### 新增

- 自定义角色可分配给用户：删除 `ASSIGNABLE_ROLES` 白名单，改为
 **校验角色在 `roles` 表中真实存在**。角色从代码常量变成数据——
 新建一个角色，用户表单立刻就能分配，无需改代码
- 角色名归一化为单一数据源 `normalize_role_name`（trim + 小写 + 非空 + 不超列宽 +
 无控制字符），新建/更新角色、创建/更新用户、追加角色五条路径共用
- 前端用户表单的角色下拉改由 `GET /admin/roles` 驱动（此前写死两个选项），
 且**懒加载**：只在打开新建/编辑弹窗时才请求，避免让只浏览列表的账号
 被迫具备 `system:role:list`
- 迁移 `008_normalize_role_names.sql`：归一化存量角色名。PR-1 的角色管理页
 已能建角色，那些非 canonical 的名字（如 `Auditor`）若不迁移，
 PR-2 上线即"这些角色突然不能分配了"
- 前端角色管理页可写：新建 / 编辑 / 删除 + **菜单与权限码授权树**
 （`type='button'` 的节点即权限码，在同一棵树里勾选）
- `roleApi` 补 `create/update/delete/assignMenus`（后端能力此前无前端入口）
- 内置角色（admin/user）在列表中标记且**不渲染删除按钮**——后端必然返回 400，
 不该给用户一个注定失败的按钮
- 前端测试 36 → 69：API 请求体契约、授权树辅助逻辑（抽到 `utils/menu.ts` 以便单测，
  项目无 `@vue/test-utils`）、内置角色名单对着后端 `BUILTIN_ROLES` 的跨端契约测试

#### PR-3：撤掉角色闸门，改用权限码包含关系作为授权下界

- **授权下界**（`PermissionGuard::ensure_covers` / `ensure_can_grant_roles`）：
  **能授予的权限码，必须全部是自己已持有的**。判定依据是权限码集合包含关系
  而非角色名，与 `find_permission_codes` 同源，**不需要新增权限码或种子数据**。
  覆盖七条写路径：`create_user` / `update_user`（目标角色与新角色两道）/
  `assign_user_role` / `reset_user_password` / `delete_user` /
  `batch_delete_users` / `toggle_user_status`
- 类型化守卫改为携带 `PermissionGuard`（`perm.guard()`），
  handler 在提取阶段鉴权之外还能再做"能否授予他人权限"的判定
- 契约测试 `every_authority_delegating_handler_checks_the_superset_rule`：
  扫描 controller 源码，上述写路径漏接授权下界即测试失败
- 契约测试 `the_role_gate_stays_removed`：注释里的迁移说明不算调用点，
  防止角色闸门被下一个人当"更安全的兜底"接回去

### 变更

- **删除 `require_role("admin")` 角色闸门**及 router 里 5 处挂载：
  "能不能进管理接口"完全由各 handler 的权限码守卫决定。
  v0.4.0 遗留的"非 admin 角色即使持有权限码也被整体挡住"由此解除
- 闸门一撤，AND 语义随之消失——**创建用户即等于授予管理员**。
  动手前先核查过：38 个 `/api/admin/*` handler **本来就都带 `_perm: Perm`**
  （此前"36 个缺失"是扫描脚本的正则 bug），真正的风险是上面那条提权面，
  由授权下界补上
- `assign_role_menus` 只拦**自授**（授给自己所属角色），不拦授予别的角色：
  admin 造新权限码再分发正是"权限码即数据"的核心工作流，一并禁掉会让权限码
  退化成只能读不能写。间接路径仍由 `ensure_can_grant_roles` 的包含关系闭合
- `update_menu` 保留严格守卫：`menus.permission` 本身就是权限码，
  改写一个已授权按钮的 permission 等于让"角色→菜单→码"这条链当场在自己身上生效，
  既不需要 `menu:grant` 也不需要新建菜单
- `AssignRoleRequest.user_id`（与路径重复的必填字段）改为可选：
  它曾让漏传请求得到 422 而非 403，等于把接口入参结构反馈给无权限调用者——
  与 v0.4.0「鉴权早于入参校验」的原则冲突。既有客户端（含前端）不受影响
- 授权树用**全量菜单树**渲染，把 `GET /admin/menus?role_id=` 的结果仅用作默认勾选值。
  过滤后的树会把父节点未授权的子节点上浮，父子关系失真会让 `cascade` 连带勾上
 本不该勾的节点，造成静默扩权
- `assign_role_menus` 是**全量覆盖**语义：未包含的既有授权会被撤销。
  弹窗提示已写明，并说明「取消某个按钮会使其上级菜单变为半选、不再随保存提交」
- 顺带修 v0.4.0 遗留缺口：菜单页的三个行内入口与「新增根菜单」此前完全没接权限码
- 授权树限高 46vh 内部滚动（全量树 40+ 节点会把弹窗顶出视口，保存按钮够不到）

### 测试

- 后端单测 32 → 44，集成测试 25 → 34；每处修复都**注入缺陷验证过测试真的会失败**
 （PR-2 注入两处：去掉角色存在性查库 → 查库证实库里真的出现 0 角色的用户；
 去掉内置角色改名守卫 → 守卫用例失败且 `admin` 被真的改名）
- **升级路径实测**：用 PR-1 的真实二进制产出真实 PR-1 库（含 4 类存量角色名），
 再用本版二进制启动同一个库，确认只有该归一化的被归一化、冲突与全空白名字
 原样保留、`role_menus` / `menus` / `user_roles` 逐项未变，且升级后
 自定义角色可正常分配
- Chrome 端到端 36/36（PR-1）+ 9/9（PR-2）通过（真实 headless Chrome + CDP 直连，零依赖）
- PR-1 的 36 项覆盖 cascade 勾父带子、重新打开授权弹窗正确回显、取消勾选后数据库真的撤销、
  撤销权限码后按钮从 DOM 移除且接口 403、删除角色后授权级联清理无残留
- PR-2 的 9 项覆盖：角色下拉由接口驱动（新建角色立刻可选）、自定义角色分配落库、
  列表显示真实角色名、编辑时角色正确回显、仅改其他字段不会丢掉角色

- **PR-3**：后端单测 44 → 49，集成测试 34 → 44，前端 69 → 81；
  注入缺陷三处（重置高权限账号口令、自授全部权限码、改写已授权按钮的
  permission），摘掉守卫后用例如期失败，确认拦住的确实是守卫而非数据库
- **PR-3 升级路径实测**（v0.4.0 → v0.5.0，真实二进制 + 真实存量库）：
 用 v0.4.0 二进制产出真实 v0.4.0 库（迁移 001–007、42 菜单、28 权限码），
 再用本版二进制启动同一个库，确认迁移 `008` 正常应用、
 42 菜单 / 28 权限码 / 侧栏 14 菜单逐项未变，admin 全部管理接口 200 **无 403 回归**；
 同一请求在 v0.4.0 下是 403（`require_role(admin)` 拦的），升级后 200 —— 闸门确实拆掉了；
 而持 `system:user:create` 的非 admin 角色建 admin 用户被 403 拦住并点名缺失的码，
 `role=user` 仍 200，提权路径确认闭合

### 已知限制（本版未解决）

- 用户表单是单角色语义（提交即整体替换角色集合），`POST /users/:id/roles` 是追加语义，
  多角色用户经用户表单保存会丢掉其余角色
- 某按钮的 permission 被清空后，该权限码**只能靠新建按钮恢复**——
  授权下界不允许把别的菜单改指成自己未持有的码。这是该规则的固有代价，
  管理员在界面上"清空权限码"后想反悔会比较绕
- `system:monitor:export`、`system:test:access` 后端已有权限码，但前端暂无入口
- 接口耗时统计保存在进程内，多副本不聚合、重启丢失
- 审计日志未实现自动保留/清理策略

## [0.4.0] - 2026-10-01

主题：**把 `menus.permission` 从元数据变成真正的权限码——后端接口级强制授权，前端按码判定。**

### 新增

- **权限码单一数据源** `src/model/permission.rs`：28 个权限码以 const 定义，
 既被 handler 引用、又驱动启动时的种子插入，杜绝"接口声明的码"与"种子里写的码"两处漂移
- **接口级强制授权**：38 个 `/api/admin/*` handler 各自声明所需权限码，未授权即 403；
 新增 `GET /api/auth/permissions` 下发当前用户的权限码
- **类型化权限提取器** `src/middleware/permission.rs`：宏 `permission_guards!` 生成
 `PermUserList` 等提取器，在**提取阶段**完成校验
- 前端 `v-permission` 指令与 `PermissionButton` 改为按权限码判定（不再按角色）；
 新增 fail-closed 的权限码 store（拉取失败即清空，宁可少显示不可多显示）
- 权限码种子：28 条 `type='button'` 的菜单行，随「菜单管理」页面天然成树
- 集成测试覆盖"撤销后立即 403""鉴权早于入参校验""授权接口自身受保护"，
 以及扫描 handler 源码的契约测试（新增管理接口漏接权限码会直接失败）
- 前后端权限码契约测试（前端遗漏或写错权限码会被拦截）

### 修复

- **无权限 + 畸形请求体返回 422 而非 403**：原校验写在 handler 函数体内，而 axum 先执行
 `Json<T>` 提取，等于把接口参数结构反馈给了无权限方。现校验前移到提取器，鉴权严格早于入参校验
- **每次启动都会悄悄恢复管理员被撤销的权限**：种子原先对所有权限码补授权。
 现仅对**本次新建**的权限码授予 admin，已实测"撤销后重启，撤销保持"

### 变更

- 权限码保存在 `menus.permission`（`type='button'` 的菜单行），经既有 `role_menus` 授权，
  **不新建权限表**、不引入第二套授权数据源
- 存量库升级已用 v0.3.0 真实二进制验证：迁移 `007` 正常应用，新增 28 条按钮行，
  **原有菜单零改动**，admin 自动获得全部权限码，管理接口无 403 回归
- 种子策略拆分：菜单树仍"仅 `menus` 为空才写"（尊重管理员的增删），权限码则
 **无条件幂等补齐**（v0.3 升级上来的库 `menus` 非空但没有按钮行）
- `/api/dict/{code}/items` **刻意不设权限码**：普通页面的 DictSelect 依赖它，
 加了会让非管理员的字典下拉全部失效
- 迁移 `007`：`menus.permission` 部分唯一索引；建索引前主动检查重复并抛可操作的错误
 （刻意不自动去重——静默去重会掩盖"菜单被手工改坏"这件事）
- 权限码不做缓存：每次受保护请求一次索引 JOIN，撤销立即生效，避免 TTL 窗口内的越权

### 已知限制（本版未解决）

- 权限码只细化 admin 路由：非 admin 角色仍被 `require_role("admin")` 整体挡住，
 暂不能仅凭权限码访问管理接口
- `role='admin'` 仍是粗粒度超级判断，未与权限码体系合并
- `system:monitor:export`、`system:test:access` 后端已有权限码，但前端暂无入口
- 接口耗时统计保存在进程内，多副本不聚合、重启丢失
- 审计日志未实现自动保留/清理策略

## [0.3.0] - 2026-09-17

主题：**补上 v0.2.0 里明确列出的两项"声明与实现差距"——OpenAPI 代码生成与菜单驱动的前端动态路由。**

### 新增

- **OpenAPI 规范改由代码生成**：引入 utoipa，为全部 44 个 handler 加 `#[utoipa::path]`
  注解、为 DTO 派生 `ToSchema`，规范在运行时生成；删除 512 行手写 JSON
- **当前用户菜单接口** `GET /api/auth/menus`：返回该用户所有角色关联的可见非按钮菜单树
- **菜单驱动的前端动态路由**：登录后按菜单树用 `import.meta.glob` 解析页面组件并注册路由，
  侧栏完全由后端菜单渲染；菜单增删不再需要改前端路由表
- **菜单种子数据**：14 条内置页面菜单 + `admin` 全量、`user` 通用页面授权；
  全新库与 v0.2 升级上来的空 `menus` 表都会在启动时自动补齐
- 文档双向覆盖测试：新增"实现里有、文档里没有"的反向检查（解析路由注册源码）
- 前端测试：菜单→路由转换、动态路由注册/撤销、种子 component 与页面文件的一致性契约

### 修复

- 手写文档同时声明 `servers: /api` 与 `/api/...` 路径，Swagger UI 的 Try it out
  会请求 `/api/api/health`（双前缀）；生成版不设 servers，路径正确
- 路由模板参数名与 handler 提取名不一致（`/users/{id}/roles`、`/roles/{id}/menus`）
- 递归菜单结构生成文档时无限展开导致栈溢出（`MenuNode` 加 `#[schema(no_recursion)]`）

### 变更

- 前端不再保留静态业务路由表：页面路径、标题、图标、层级全部来自后端菜单
- `component` 字段约定为 `src/views` 下的相对路径（如 `system/user/index`），
  解析不到页面文件时会跳过并告警，且由前端契约测试提前拦截
- 新增后端依赖 `utoipa` 5.5

### 已知限制（本版未解决）

- 按钮级权限仍为基于角色的显隐；`menus.permission` 目前只是元数据，未接入接口级权限码校验
- 接口耗时统计保存在进程内，多副本不聚合、重启丢失
- 审计日志未实现自动保留/清理策略
- 前端 50 个文件未统一 Prettier 格式化（未纳入 CI 门禁）

## [0.2.0] - 2026-09-17

主题：**闭环与可部署基线**。不新增业务模块，把「声明了但未生效」的能力补齐或删除，
并建立可回归的验证边界。

### 新增

- 启动时自动执行数据库迁移（`sqlx::migrate!` 内嵌迁移，`MIGRATE_ON_STARTUP` 可关闭），空库可直接启动
- `/api/health` 真实探测数据库与 Redis，依赖异常返回 503 并给出组件状态
- JWT 增加 `jti`：登出只注销当前令牌，不再影响该用户其他设备
- 用户级会话吊销：改密 / 停用 / 删除账号 / 变更角色后，存量令牌立即失效
- 登录爆破防护：账号 + 客户端 IP 双维度失败计数，超阈值返回 429
- `TRUST_PROXY_HEADERS` 开关决定是否采信 `X-Forwarded-For`，默认取 TCP 真实来源
- 审计日志真正接线：受保护请求写入 `audit_logs`（含 `request_id` 关联日志）
- 字典读取接口 `/api/dict/{code}/items`，对任意已登录用户开放
- 集成测试（`tests/api_integration.rs`，覆盖迁移/会话/权限/一致性/文档契约）
- 前端单元测试（Vitest）与 ESLint 9 扁平配置
- `scripts/test_env.sh`：无需 Docker 即可拉起本地集成测试依赖

### 修复

- **登出后重新登录会让已注销令牌复活**：黑名单按用户 ID 存储且登录时清空，改为按 `jti` 存储
- **角色双数据源**：`users.role` 与 `user_roles` 并存导致"改角色不改权限"，现以 `user_roles` 为唯一来源
- **用户列表角色恒为空**：`UserInfo::from` 丢弃角色列表，改为由角色集合推导主角色
- **接口返回 `Admin/User` 而非 `admin/user`**：`Role` 增加 `serde(rename_all = "lowercase")`，与前端类型一致
- **改字典后最长 1 小时读到旧值**：缓存失效此前是空实现，现真正删除对应缓存键
- **删除字典项失败被静默吞掉**：`execute().ok()` 改为返回错误（不存在时 404）
- **批量删除/角色分配非事务**：删除交由外键级联，角色替换在单事务内完成
- **可删除最后一名管理员 / 删除自己**：补充守卫（含批量删除整批判断）
- **500 无任何服务端日志**：`InternalServerError` 现在会记录具体原因
- **`ValidationFailed` 返回 401**：拆分为 `InvalidCredentials`(401) 与 `ValidationFailed`(400)
- **端口不一致**：前端开发代理默认从 9527 修正为与后端一致的 8080
- **Docker HEALTHCHECK 无效**：此前是"再启动一个服务进程"，改为探测 `/api/health`
- **Redis 故障时请求挂起**：为连接管理器设置重试/连接/响应超时，改为快速 503
- **`pnpm lint` 从未可用**：`.eslintrc.cjs` 是 ESLint 8 格式，迁移为 ESLint 9 扁平配置
- **`/api/*` 未知路径返回 401**：鉴权中间件由 `layer` 改为 `route_layer`，未知路径正确返回 404

### 安全（删除无实际作用的模块）

- 删除请求体加密 `CryptoService` 及 `CRYPTO_*` / `RSA_*` 配置：从未接入任何中间件，属死能力
- 删除验证码中间件：只校验答案位数、不与 token 比对，等价于不设防（`generate_captcha` 也无调用方）
- 删除 SQL 注入关键词黑名单中间件：SQLx 已是参数化查询，该中间件会误拦合法内容且覆盖不到 query string
- 口令传输模型统一为「HTTPS 明文 → 服务端 Argon2」；前端移除 SHA-256 预哈希
- "记住密码"不再把口令写入 localStorage，只记忆用户名
- 生产环境拒绝示例占位 `JWT_SECRET` 与通配符 CORS 来源

### 变更（破坏性）

- 迁移 `006` 删除 `users.role` 列（先把数据回填进 `user_roles`）
- 口令格式变更：存量账号在下次登录时自动透明升级（Argon2(sha256) → Argon2(明文)）
- 新增 `jti` 后，v0.1 签发的令牌失效，需重新登录
- 字典读取路径 `GET /api/admin/dict/{code}/items` → `GET /api/dict/{code}/items`
- 移除 `PUT /api/admin/users/{id}/roles`（非事务、吞错且未被前端使用）
- 移除配置：`DATABASE_READ_URL`、`DB_READ_POOL_MAX_SIZE`、`CRYPTO_*`、`RSA_*`、`CAPTCHA_ENABLED`、`APP_SECRET`
- `REDIS_URL` 由"可选"改为必需（此前文档标注可选，实际启动即强依赖）
- `/api/health` 返回体新增 `data.{status,database,redis}` 字段

### 删除的死代码

`CrudTemplate`（拼接 SQL）、读写分离连接池（读库从未被使用）、Redis 高级缓存辅助函数、
分页 `keyword` 相关字段、未使用的 `validator` 依赖等。clippy 警告数 **35 → 0**。

### 已知限制（未在本次范围内）

- OpenAPI 仍为手写规范，尚未由代码生成（已有"文档路由必须真实存在"的集成测试兜底）
- 前端导航仍为静态路由表，未由后端菜单数据驱动
- 按钮级权限为基于角色的显隐，无独立权限码体系
- 接口耗时统计保存在进程内，多副本不聚合、重启丢失
- 审计日志未实现自动保留/清理策略
- 前端 50 个文件未经过 Prettier 统一格式化（未纳入 CI 门禁）
