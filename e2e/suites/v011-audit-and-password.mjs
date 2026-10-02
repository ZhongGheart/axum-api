// v0.11.0 端到端回归：登录可审计 + 自助改密 + 口令策略 + 受限令牌
//
// 集成测试证明的是"接口正确"。这里证明的是**界面 + 接口 + 数据库**合起来自洽，
// 补四条集成测试天然看不见的东西：
//
//  1) 登录审计在**系统日志页真的看得见**（不是只在库里躺着）
//  2) 登录失败也落审计，且 action 能把成功/失败分开（"有没有人在爆破"）
//  3) 受限令牌在**界面**上被拦住：管理员建号 → 该账号只能待在个人中心
//  4) 改密走**界面表单**，改完真的被登出、真的能用新口令登录

import { Session, waitFor } from '../lib/harness.mjs'

const s = new Session('v0.11.0 登录审计与自助改密')
await s.start()

const uniq = 'v011' + Math.random().toString(36).slice(2, 8)
const wait = (ms) => new Promise((r) => setTimeout(r, ms))

// 造出来的人名/口令要能过复杂度策略（至少 8 位 + 两类字符），
// 否则第一步建号就会被 400 挡掉，后面每条断言都会因前提不成立而假绿
const newbieName = uniq + '_newbie'
const newbieEmail = newbieName + '@example.com'
const initialPass = 'Init1pass!'
const changedPass = 'New1pass!'

s.log('\n[1] 真实登录 admin')
await s.login()
s.check('登录 admin', true)

// ── 受限令牌：管理员建号 → 该账号只能待在个人中心 ──────────────

s.log('\n[2] 管理员建号（会置"强制改密"）')
const created = await s.api('POST', '/api/admin/users', {
  username: newbieName,
  email: newbieEmail,
  password: initialPass,
  // 种子里只有 admin / user 两个角色，普通用户被管理员建号才是真实场景
  roles: ['user'],
})
s.check(
  '管理员建号成功',
  created.status === 200,
  'status=' + created.status + ' ' + JSON.stringify(created.body?.message || ''),
)

const newbieId = created.body?.data?.id

s.log('\n[3] 该账号登录后应被强制改密')
const staleTok = await s.tokenFor(newbieName, initialPass)
const meWithStale = await s.apiAs(staleTok, 'GET', '/api/auth/me')
s.check(
  '受限令牌仍可读 /me（否则用户连"我是谁"都看不到）',
  meWithStale.status === 200,
  'status=' + meWithStale.status,
)
s.check(
  '用户信息声明须改密',
  meWithStale.body?.data?.must_change_password === true,
  JSON.stringify(meWithStale.body?.data),
)

// 受限令牌打业务接口必须是 403，且**说清原因**而不是笼统的"无权限"。
// 用 /api/auth/menus 而不是 /api/admin/users：后者普通用户本来就没权限，
// 403 会有两个来源，判据就不干净了。menus 对任意登录用户开放，
// 所以这里的 403 只可能来自"受限令牌"这一个原因。
const staleBiz = await s.apiAs(staleTok, 'GET', '/api/auth/menus')
s.check('受限令牌打业务接口被拒', staleBiz.status === 403, 'status=' + staleBiz.status)
const staleMsg = String(staleBiz.body?.message || '')
s.check('拒绝理由指向改密', staleMsg.includes('密码'), JSON.stringify(staleMsg))

// ── 界面层：受限账号只能待在个人中心 ──────────────────────────

s.log('\n[4] 受限账号的界面跳转')
// **走真实登录表单**，不手工塞 localStorage：
// 受限标记的真相在登录响应与 store 里，手工塞一份 token 造出来的
// 状态和真实会话不是一回事——那样验的是"我造出来的假会话被拦住"，
// 而不是"用户真的被拦住"
await s.login(newbieName, initialPass, '/profile')

// 登录后应当主动弹到个人中心，而不是停在首页等人自己找
s.check('登录后直接落在个人中心',
  (await s.evalJs('return location.pathname')) === '/profile')

// 再手动访问一个业务页：守卫必须挡住
await s.goto('/system/user', 1500)
const forced = await s.evalJs(
  'return { path: location.pathname,'
  + ' alert: document.body.innerText.includes("需要先修改初始密码"),'
  + ' form: document.body.innerText.includes("修改密码") };',
)
s.check(
  '受限账号访问业务页被弹回个人中心',
  forced.path === '/profile',
  'path=' + forced.path,
)
s.check('个人中心说明了为什么被拦在这里', forced.alert, JSON.stringify(forced))
s.check('个人中心给出了改密表单', forced.form)
await s.shot('v011-forced-profile')

// 侧栏不该把业务菜单摊开给一个什么都做不了的人。
// 判据取**菜单项数量**而不是侧栏文本——侧栏里还有 logo 文字
// （"Axum Admin"），按文本判会恒假红
const sidebar = await s.evalJs(
  'return { items: document.querySelectorAll(".layout-sider .n-menu-item").length };',
)
s.check('受限账号侧栏没有业务菜单', sidebar.items === 0, JSON.stringify(sidebar))

// 受限用户唯一能用的页面不该刷一屏"权限不足"。
// 曾经真实发生过：路由守卫明知菜单必然 403 仍去加载，
// 一次落地弹 4 个错误提示，把真正值得看的错误全淹了。
// 界面"能用"和界面"干净"是两件事，这条守的是后者。
const toasts = await s.evalJs(
  'return (document.body.innerText.match(/权限不足/g) || []).length;',
)
s.check('个人中心没有"权限不足"提示刷屏', toasts === 0, '出现 ' + toasts + ' 次')

// ── 界面层：走真实表单改密 ────────────────────────────────────

s.log('\n[5] 弱口令应被界面当场拦下')
await s.setInput('请输入当前登录密码', initialPass)
await s.setInput('至少 8 位', 'abcdefgh')
await wait(400)
const weakState = await s.evalJs(
  'const btn = [...document.querySelectorAll("button")]'
  + '  .find(b => b.innerText.replace(/\\s+/g, "").includes("确认修改"));'
  + ' return { disabled: btn ? btn.disabled : null,'
  + ' err: document.body.innerText.includes("两类") };',
)
s.check(
  '弱口令时提交按钮禁用',
  weakState.disabled === true,
  JSON.stringify(weakState),
)
// 表单在折叠线以下，不滚过去截图里只有页面顶部，
// 人工复核等于没看见那条规则提示
await s.evalJs(
  'const el = document.querySelector(".n-form");'
  + ' if (el) el.scrollIntoView({ block: "center" });'
  + ' await new Promise(r => setTimeout(r, 300));'
  + ' return true',
)
await s.shot('v011-weak-password')

s.log('\n[6] 正常改密')
await s.setInput('至少 8 位', changedPass)
await s.setInput('再次输入新密码', changedPass)
await wait(400)
await s.clickByText('确认修改')
// 等**可观测的条件**，不要固定 sleep：改密要跑两次 Argon2 哈希再吊销全部会话，
// 单跑时约 1s，全量跑时机器更满、可能好几秒。
// 固定等待在慢机器上会假红——曾经就是这样：按钮还在转圈就断言了
await waitFor(
  () => s.evalJs('return location.pathname === "/login"'),
  { timeout: 30000, label: '改密后跳回登录页' },
)
await wait(500)

// 改密后端吊销了全部会话，前端应主动登出并回到登录页。
// 若页面还停在个人中心，说明前端没接住"会话已被吊销"这件事
const afterChange = await s.evalJs(
  'return { path: location.pathname,'
  + ' token: localStorage.getItem("axum_token") };',
)
s.check('改密后回到登录页', afterChange.path === '/login', 'path=' + afterChange.path)
s.check('改密后本地令牌已清', !afterChange.token, 'token=' + afterChange.token)
await s.shot('v011-after-change')

s.log('\n[7] 新口令可用、旧口令失效')
const newTok = await s.tokenFor(newbieName, changedPass)
const withNew = await s.apiAs(newTok, 'GET', '/api/auth/me')
s.check('能用新口令登录', withNew.status === 200, 'status=' + withNew.status)
s.check(
  '改密后不再被强制改密',
  withNew.body?.data?.must_change_password === false,
  JSON.stringify(withNew.body?.data),
)

// 放行侧：同一个接口，改密后必须放行——
// 只测拒绝的话，一个"改完密仍然锁死"的实现也能全绿
const newBiz = await s.apiAs(newTok, 'GET', '/api/auth/menus')
s.check('新令牌可正常调用同一个业务接口', newBiz.status === 200, 'status=' + newBiz.status)

let oldRejected = false
try {
  await s.tokenFor(newbieName, initialPass)
} catch {
  oldRejected = true
}
s.check('旧口令已失效', oldRejected)

// 上面两次是**故意**造出来的失败（受限令牌 403、旧口令 401）。
// 就地把它们从账上划掉，而不是把 401/403 加进末尾的白名单——
// 白名单会让此后任何"令牌意外过期""会话被误吊销"都不再是信号，
// 而那恰恰是这版最该抓的回归。
s.forgetDeliberateFailures()

// ── 登录审计：成功与失败都要看得见 ────────────────────────────

s.log('\n[8] 切回 admin 看审计')
await s.login()
await wait(1500)

const successRows = await s.api(
  'GET',
  '/api/admin/audit-logs?action=AUTH_LOGIN_SUCCESS&username=' + newbieName,
)
const successItems = successRows.body?.data?.items || []
s.check(
  '登录成功落审计',
  successItems.length > 0,
  'total=' + successRows.body?.data?.total,
)

const failRows = await s.api(
  'GET',
  '/api/admin/audit-logs?action=AUTH_LOGIN_FAILURE&username=' + newbieName,
)
const failItems = failRows.body?.data?.items || []
s.check(
  '登录失败落审计',
  failItems.length > 0,
  'total=' + failRows.body?.data?.total,
)

// 审计条目必须**区分成功与失败**，否则事后无法回答"有没有人在爆破"
const bothActions = await s.evalJs(
  'const tok = JSON.parse(decodeURIComponent(atob(localStorage.getItem("axum_token")))).value;'
  + ' const r = await fetch("/api/admin/audit-logs?action=AUTH_LOGIN&page_size=200",'
  + ' { headers: { Authorization: "Bearer " + tok } });'
  + ' const j = await r.json();'
  + ' return (j.data?.items || []).map(x => x.action);',
)
s.check(
  '成功与失败是两种不同的 action',
  bothActions.includes('AUTH_LOGIN_SUCCESS') && bothActions.includes('AUTH_LOGIN_FAILURE'),
  [...new Set(bothActions)].join(',') || '（一条都没有）',
)

// client_ip 不能是空的：Redis 计数器带 TTL 会过期，
// 而"谁从哪尝试登录"正是审计要留下来的东西
const withIp = successItems.filter((x) => !!x.client_ip)
s.check(
  '登录审计带 client_ip',
  successItems.length > 0 && withIp.length === successItems.length,
  withIp.length + '/' + successItems.length,
)

// 改密**也要**落审计：它挂在 auth_middleware 之下，中间件会记下
// `PUT /api/auth/password`。这不是"多了一个动作"，
// 而是三个语义动作（登录成功/失败/注册）之外，走中间件的常规路径。
const pwdActions = await s.api(
  'GET',
  '/api/admin/audit-logs?action=PASSWORD&page_size=50',
)
const pwdItems = (pwdActions.body?.data?.items || []).filter(
  (x) => x.path === '/api/auth/password' && x.status_code === 200,
)
s.check(
  '改密成功也落审计',
  pwdItems.length > 0,
  'total=' + pwdActions.body?.data?.total,
)

// 审计里绝不能出现明文口令。
// 中间件只记 query string、不记 body，因此 params 应当为空——
// 一旦哪天有人"顺手"把 body 也塞进审计，这个断言立刻变红
const leaked = pwdItems.filter(
  (x) => JSON.stringify(x).includes(initialPass) || JSON.stringify(x).includes(changedPass),
)
s.check('审计里没有明文口令', leaked.length === 0,
  leaked.length + ' 条含口令')

s.log('\n[9] 系统日志页看得到登录记录')
await s.goto('/system/log', 1500)
await s.setInput('操作', 'AUTH_LOGIN_SUCCESS')
await s.clickByText('查询')
await wait(1500)
const inUi = await s.evalJs(
  'const rows = [...document.querySelectorAll(".n-data-table-tbody .n-data-table-tr")]'
  + '  .map(tr => [...tr.querySelectorAll("td")].map(td => td.innerText.trim()));'
  + ' return { total: rows.length,'
  + ' withPath: rows.filter(r => r.some(c => c.includes("/api/auth/login"))).length };',
)
s.check(
  '系统日志页能筛出登录成功记录',
  inUi.total > 0 && inUi.withPath === inUi.total,
  JSON.stringify(inUi),
)
await s.shot('v011-login-audit-ui')

s.log('\n[10] 无残留')
if (newbieId) {
  const del = await s.api('DELETE', '/api/admin/users/' + newbieId)
  s.check('临时账号已删除', del.status === 200, 'status=' + del.status)
}

s.checkNoConsoleErrors([])
const failed = s.summary()
await s.stop()
process.exit(failed > 0 ? 1 : 0)
