// v0.15.0 界面自查：会话失效时，界面说的是"404 页面未找到"
//
// 令牌被服务端吊销后，用户看到的是"404 页面未找到"+ 同一条提示弹 4 次 +
// 令牌留在 localStorage。三件事都是真实产品操作会走到的路径：
// 改角色、改密、停用账号都会吊销会话。
//
// 每条断言都盯一个具体的骗法。反向判据（不该发生什么）比正向更有价值——
// 只断言"跳到登录页了"的话，一个把所有 401 都当会话失效的实现也能过。

import { Session, sleep, waitFor } from '../lib/harness.mjs'

const s = new Session('v0.15.0 会话失效时界面不说谎')

await s.start()

/**
 * 真正落到登录页
 *
 * 不能直接 `goto('/login')`：只要 localStorage 里还有令牌，路由守卫就会把
 * `/login` 弹回首页（防的是"已登录用户又看到登录页"）。于是页面停在 `/`，
 * 而 `/` 上本来就没有提示条——**"登录页没有会话失效提示"这条断言会因此
 * 空过**，套件全绿而要验的东西一次都没验到。
 * 本轮第一版就踩了这个坑，是套件自己报出来的。
 */
async function landOnLogin() {
  await s.goto('/login', 300)
  // **只清 localStorage，不动 sessionStorage**：后者存着"会话为何结束"，
  // 而"主动登出不该留下会话失效提示"这条断言要验的正是它有没有被错写进去。
  // 在这里顺手清掉，等于把要验的证据先销毁，断言又会空过。
  await s.evalJs('localStorage.clear(); return true')
  await s.goto('/login', 800)
  await waitFor(() => s.evalJs('return document.querySelectorAll("input").length >= 2'),
    { label: '登录表单' })
}

// ── 1. 正常登录：提示不得常驻 ─────────────────────────────

s.log('\n[1] 正常登录后进登录页，不该出现会话失效提示')
await s.login()
s.check('admin 登录成功', (await s.evalJs('return location.pathname')) === '/')

await s.goto('/system/user', 1200)
s.check('用户管理页可进入',
  (await s.evalJs('return location.pathname')) === '/system/user',
  await s.evalJs('return location.pathname'))

// 带着令牌访问 /login 会被弹回首页，所以先落地到登录页再断言
await landOnLogin()
s.check('确实落在登录页上（否则下面两条都是空过）',
  (await s.evalJs('return location.pathname')) === '/login',
  await s.evalJs('return location.pathname'))
const noAlert = await s.evalJs(
  'const a = [...document.querySelectorAll(".n-alert")];'
  + ' return a.length === 0 || !a.some(x => /失效|注销|过期/.test(x.innerText))'
)
s.check('主动登出后登录页没有"会话已失效"提示（提示不是常驻的）', noAlert === true,
  noAlert ? '未出现' : '登出也报了会话失效')

// ── 2. 会话被吊销：界面必须说对 ──────────────────────────

s.log('\n[2] 会话被吊销后访问业务页')
await s.login()
const revoked = await s.currentToken()
const out = await s.apiAs(revoked, 'POST', '/api/auth/logout')
s.check('吊销会话成功（真实走 /auth/logout）', out.status === 200, 'status=' + out.status)

const me = await s.apiAs(revoked, 'GET', '/api/auth/me')
s.check('同一令牌再调 /api/auth/me 返回 401', me.status === 401, 'status=' + me.status)
// 后端把 401 的三种原因分得很清楚，这是界面应当转述的原话
s.check('401 带的是后端自己的原因', /注销|失效|过期/.test(me.body?.message || ''),
  me.body?.message)

await s.goto('/system/user', 2200)

const path = await s.evalJs('return location.pathname')
s.check('被弹回登录页', path === '/login', 'pathname=' + path)
s.check('**不得**停在原业务页', path !== '/system/user', 'pathname=' + path)

const body = await s.evalJs('return document.body.innerText.replace(/\s+/g, " ").trim()')
s.check('页面**不得**出现"404 页面未找到"（方向性错误的诊断）',
  !body.includes('404 页面未找到'), body.slice(0, 90))
s.check('登录页解释了下为什么会到这里', /失效|注销|过期/.test(body), body.slice(0, 120))

const reasonText = await s.evalJs(
  'const a = [...document.querySelectorAll(".n-alert")].map(x => x.innerText.replace(/\s+/g," ").trim());'
  + ' return a.join(" | ")'
)
s.check('解释用的是后端给的原因，而不是前端通用句',
  /令牌已被注销/.test(reasonText), reasonText || '没有提示条')

const toasts = await s.evalJs(
  'return [...document.querySelectorAll(".n-message")].map(x => x.innerText.trim())'
)
s.check('401 没有刷出多条重复提示', toasts.length <= 1,
  toasts.length + ' 条: ' + JSON.stringify(toasts))

const tok = await s.evalJs(
  'const t = localStorage.getItem("axum_token"); return t ? "有" : "无"'
)
s.check('localStorage 里的令牌已被清除', tok === '无', '令牌=' + tok)

const menus = await s.evalJs(
  'const t = localStorage.getItem("axum_token");'
  + ' return t ? "还在" : "已清"'
)
s.check('（复核）令牌键确实不在了', menus === '已清', menus)
await s.shot('v015-session-ended')

// ── 3. 原因取走即清除：重新登录不得再显示 ────────────────

s.log('\n[3] 重新登录后不该残留旧提示')
await s.login()
s.check('可以重新登录', (await s.evalJs('return location.pathname')) === '/')

await s.goto('/system/user', 1200)
s.check('重新登录后业务页可用',
  (await s.evalJs('return location.pathname')) === '/system/user',
  await s.evalJs('return location.pathname'))

await landOnLogin()
s.check('（复核）确实落在登录页上',
  (await s.evalJs('return location.pathname')) === '/login',
  await s.evalJs('return location.pathname'))
const noStale = await s.evalJs(
  'const a = [...document.querySelectorAll(".n-alert")].map(x => x.innerText);'
  + ' return !a.some(t => /令牌已被注销/.test(t))'
)
s.check('旧原因没有被带到第二次登录（取走即清除）', noStale === true,
  noStale ? '未残留' : '残留了上一轮的会话失效提示')

// ── 4. 口令错误：不得被当成会话失效 ──────────────────────

s.log('\n[4] 登录页输错口令')
await landOnLogin()
// 起点必须干净：上一轮若有残留，这里测的就不是"口令错误会不会误报会话失效"
const cleanStart = await s.evalJs(
  'const a = [...document.querySelectorAll(".n-alert")].map(x => x.innerText);'
  + ' return a.length === 0'
)
s.check('进入口令错误测试前登录页是干净的', cleanStart === true,
  cleanStart ? '干净' : '有残留提示条')
await s.setInput('请输入用户名', 'admin')
await s.setInput('请输入密码', 'definitely-not-the-password')
await s.clickByText('登 录')
await sleep(1800)

const badPath = await s.evalJs('return location.pathname')
s.check('口令错误时停在登录页（不得被弹走）', badPath === '/login', 'pathname=' + badPath)

const badToasts = await s.evalJs(
  'return [...document.querySelectorAll(".n-message")].map(x => x.innerText.trim())'
)
s.check('提示说的是"用户名或密码错误"，不是"未授权，请重新登录"',
  badToasts.some((t) => /用户名或密码错误/.test(t)),
  JSON.stringify(badToasts))
s.check('口令错误不得报成"会话已失效"（那是另一件事）',
  !badToasts.some((t) => /会话已失效|令牌已被注销/.test(t)),
  JSON.stringify(badToasts))
s.check('口令错误只提示一次，不重复弹', badToasts.length <= 1,
  badToasts.length + ' 条: ' + JSON.stringify(badToasts))

// ── 5. 无残留 ────────────────────────────────────────────

// 本套件只吊销自己的会话，不建角色/账号
const leftover = await s.evalJs(
  'const raw = sessionStorage.getItem("axum_session_end_reason");'
  + ' return raw === null ? "已取走" : "残留: " + raw'
)
s.check('会话结束原因已被登录页取走', leftover === '已取走', leftover)

s.forgetDeliberateFailures()
s.checkNoConsoleErrors([])
s.checkNoUnexpectedHttp('除故意吊销外无意外 4xx/5xx', ['401'])

const failed = s.summary()
await s.stop()
process.exit(failed > 0 ? 1 : 0)
