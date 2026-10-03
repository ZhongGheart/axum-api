// v0.18.0 端到端回归：创建账号的两个入口不许教用户填一个后端不收的值
//
// 这一版的整条主线是"表单不许说谎"。v0.11.0 把口令策略从"至少 6 位"
// 收紧为"至少 8 位 + 两类字符"，却只迁移了改密页——注册页与管理员建号
// 对话框整整七版没人碰，各自留着 v0.10 时代的 `min: 6`。
//
// 集成测试证明的是"后端会拒"。这里证明的是**界面在用户点提交之前就说了**，
// 而且——这才是关键——**请求根本没发出去**。
//
// 为什么非要卡"没发出去"：如果只断言页面上出现一句错误提示，
// 那么"前端放行 → 后端 400 → 界面弹报错"也能满足它。
// 用户仍然是填完整张表才被告知规则是什么，而这正是要修的缺陷本身。
// harness 会记录所有 >=400 的响应，所以"表单被前端拦下"与
// "请求打到了后端"是可区分的。

import { Session, waitFor } from '../lib/harness.mjs'

const s = new Session('v0.18.0 账号表单规则')
await s.start()

const uniq = 'v018' + Math.random().toString(36).slice(2, 8)
const wait = (ms) => new Promise((r) => setTimeout(r, ms))

/** 界面上所有输入框的 placeholder，用来定位字段 */
const placeholders = () => s.evalJs(
  'return [...document.querySelectorAll("input")].map(i => i.placeholder || "");',
)

/** 页面正文里有没有出现某个提示（表单错误是 naive-ui 渲染的 feedback） */
const pageHas = (text) => s.evalJs(
  'return document.body.innerText.includes(' + JSON.stringify(text) + ');',
)

/** 只填一个字段，不动其余字段——用占位符前缀定位，避免重名 */
async function fill(fieldPrefix, value) {
  await s.setInput(fieldPrefix, value)
}

/** 读输入框的 maxlength；null 表示没有这个属性 */
function maxLengthOf(prefix) {
  return s.evalJs(
    'const el = [...document.querySelectorAll("input")].find(i => i.placeholder && i.placeholder.includes('
    + JSON.stringify(prefix) + '));'
    + ' return el ? (el.maxLength >= 0 ? el.maxLength : null) : null;',
  )
}

// ── 1. 注册页：口令规则 ──────────────────────────────────────────

s.log('\n[1] 注册页')
await s.goto('/register', 1200)

const regPh = await placeholders()
const regUserPh = regPh.find((p) => p.includes('下划线'))
const regPassPh = regPh.find((p) => p.includes('两类'))

s.check(
  '用户名 placeholder 说清了连字符/下划线（后端一直许这两个字符，旧文案只说"字母或数字"）',
  !!regUserPh,
  JSON.stringify(regPh),
)
s.check(
  '口令 placeholder 说的是现行策略而非"至少 6 个字符"',
  !!regPassPh && !regPh.some((p) => p.includes('至少 6')),
  JSON.stringify(regPh),
)
await s.shot('v018-register')

s.log('\n[2] 弱口令必须在**提交前**被拦下')
// 后端策略：8-128 字符 + 至少两类字符。'abcdefgh' 长度够但只有一类，
// v0.18.0 之前前端会放行它，然后用户提交后才收到 400。
const weak = 'abcdefgh'

await fill('下划线', uniq + '_weak')
await fill('邮箱地址', uniq + '_weak@example.com')
await fill('两类', weak)
await fill('请再次输入', weak)

// 清账：接下来若有 >=400 的网络响应，就说明请求真的发出去了
s.resetBadResponses()
await s.clickByText('注 册')
await wait(1200)

s.check('弱口令在页面上给出提示', await pageHas('两类'), '页面上找不到复杂度提示')
s.check(
  '弱口令没有发到后端（0 个 4xx/5xx 响应）',
  s.badResponses.length === 0,
  '出现了 ' + (s.badResponses.slice(0, 3).join(' | ') || ''),
)
s.check('没有离开注册页', (await s.evalJs('return location.pathname')) === '/register')
await s.shot('v018-register-weak-blocked')

// ── 3. 注册页：用户名字符集 ──────────────────────────────────────

s.log('\n[3] 用户名含 @ 必须在提交前被拦下')
await fill('下划线', uniq + '@name')
await fill('两类', 'Abcdef12')
await fill('请再次输入', 'Abcdef12')
s.resetBadResponses()
await s.clickByText('注 册')
await wait(1200)

s.check(
  '用户名字符集提示出现在页面上',
  await pageHas('只能包含'),
  '页面上找不到字符集提示',
)
s.check(
  '含 @ 的用户名没有发到后端',
  s.badResponses.length === 0,
  '出现了 ' + (s.badResponses.slice(0, 3).join(' | ') || ''),
)

// 第 4、5 步会造出两个真实账号（正常注册 + 多字节用户名）。
// 清理放在 finally：断言失败会直接抛出异常，写在后面的清理语句就永远不会执行——
// 而"注册没成功"恰恰是最容易失败的路径，于是每次红一次漏一个账号。
const created = []
try {
  // ── 4. 合规注册真的能走通 ───────────────────────────────────────

  s.log('\n[4] 合规取值应当被放行并真的注册成功')
  const goodName = uniq + '_ok'
  // 先登记再操作：万一填表过程就抛了，finally 仍知道该删谁
  created.push(goodName)
  await fill('下划线', goodName)
  await fill('邮箱地址', goodName + '@example.com')
  await fill('两类', 'Abcdef12')
  await fill('请再次输入', 'Abcdef12')
  s.resetBadResponses()
  await s.clickByText('注 册')
  await waitFor(
    () => s.evalJs('return location.pathname !== "/register";'),
    { timeout: 15000, label: '注册成功后离开注册页' },
  )
  // 注册接口本身返回 200，不进 badResponses；这里确认没有意外失败
  s.checkNoUnexpectedHttp('注册过程无意外失败')

  // 自注册成功后应用跳的是**登录页**，此刻浏览器里没有令牌，
  // 所以要登成 admin 才能查管理接口——直接 `s.api` 会拿着空令牌去问，必然 401。
  await s.login()
  const registered = await s.api('GET', '/api/admin/users?keyword=' + goodName)
  const rows = registered.body?.data?.items || []
  s.check(
    '新账号真的落在用户列表里',
    rows.some((u) => u.username === goodName),
    'status=' + registered.status + ' ' + JSON.stringify(registered.body?.data).slice(0, 200),
  )

  // ── 5. 多字节用户名：界面不该拦下后端放行的值 ───────────────────

  s.log('\n[5] 17 个汉字的用户名（51 字节 / 17 字符）')
  // 后端按字符计数，varchar(50) 也按字符——所以它合法。
  // 若前端改成按字节/码元判，这里会被自己人拦下。
  const chineseName = '一'.repeat(17) + uniq.slice(-3)
  created.push(chineseName)
  await s.goto('/register', 1000)
  await fill('下划线', chineseName)
  await fill('邮箱地址', uniq + '_cn@example.com')
  await fill('两类', 'Abcdef12')
  await fill('请再次输入', 'Abcdef12')
  s.resetBadResponses()
  await s.clickByText('注 册')
  await wait(1500)

  s.check(
    '多字节用户名没被字符集规则拦下',
    !(await pageHas('只能包含')),
    '页面提示了字符集问题，而后端是放行的',
  )
  const cnList = await s.api('GET', '/api/admin/users?keyword=' + encodeURIComponent(chineseName))
  const cnRows = cnList.body?.data?.items || []
  const cnCreated = cnRows.some((u) => u.username === chineseName)
  s.check('多字节用户名真的注册成功', cnCreated,
    'status=' + cnList.status + ' ' + JSON.stringify(cnList.body?.data).slice(0, 200))

  // 清理统一放在 finally（下一段）
} finally {
  for (const name of created) {
    const list = await s.api('GET', '/api/admin/users?keyword=' + encodeURIComponent(name))
      .catch(() => null)
    const items = list?.body?.data?.items || []
    for (const u of items) {
      if (u.username === name) {
        await s.api('DELETE', '/api/admin/users/' + u.id).catch(() => {})
      }
    }
  }
}

// ── 6. 管理员建号对话框：同款规则 ───────────────────────────────

s.log('\n[6] 管理员建号对话框')
await s.login()
await s.goto('/system/user', 1200)
await s.clickByText('新建用户')
await wait(1000)

const modalPh = await placeholders()
const modalUserPh = modalPh.find((p) => p.includes('下划线'))
s.check('对话框里的用户名 placeholder 与注册页同源', !!modalUserPh,
  JSON.stringify(modalPh))

// 旧版这里连 maxlength 都没有，输入框对长度毫无约束
const modalPassMax = await maxLengthOf('两类')
s.check(
  '对话框口令框有 maxlength（原先完全没有）',
  modalPassMax !== null,
  'maxlength=' + modalPassMax,
)

await fill('下划线', uniq + '_dlg')
await fill('邮箱地址', uniq + '_dlg@example.com')
await fill('两类', weak)
s.resetBadResponses()
// 对话框的提交按钮文案（见 system/user/index.vue 的 footer 插槽）
await s.clickByText('保存')
await wait(1200)

s.check('对话框内弱口令给出提示', await pageHas('两类'))
s.check(
  '对话框内弱口令没有发到后端',
  s.badResponses.length === 0,
  '出现了 ' + (s.badResponses.slice(0, 3).join(' | ') || ''),
)
await s.shot('v018-admin-create-weak-blocked')

// ── 7. 登录页：长邮箱必须能输进去 ───────────────────────────────

s.log('\n[7] 登录页标识符上限')
// users.email 是 varchar(255)，后端登录是裸查询（无长度校验）。
// 旧登录页写死 maxlength=50，于是持有长邮箱的合法用户
// 在自己的登录页上敲不进自己的邮箱。
// **必须先清令牌再进 /login**：此刻还带着第 6 步 admin 的会话，
// 路由守卫会把 /login 弹回首页，于是页面上根本没有登录输入框，
// maxLengthOf 只会返回 null —— 一个测不出"上限是 50 还是 255"的假失败。
await s.goto('/login', 300)
await s.evalJs('localStorage.clear(); return true')
await s.goto('/login', 1000)
const loginIdMax = await maxLengthOf('请输入用户名或邮箱')
s.check(
  '登录标识符 maxlength 是 255（不是 50）',
  loginIdMax === 255,
  'maxlength=' + loginIdMax,
)

// 造一个 73 字符邮箱的账号，再真的用它登录一次。
//
// 用**公开注册接口**而不是管理员建号：管理员建号会置"强制改密"，
// 登录后会被路由守卫弹去 /profile，那测的就不是"能不能用邮箱登录"了。
// 自注册账号没有那个标记，登录后正常落首页，判据才干净。
// 造一个长邮箱的账号，再真的用它登录一次。
//
// 用**公开注册接口**而不是管理员建号：管理员建号会置"强制改密"，
// 登录后会被路由守卫弹去 /profile，那测的就不是"能不能用邮箱登录"了。
// 自注册账号没有那个标记，登录后正常落首页，判据才干净。
//
// 清理放在 finally 里：断言一失败就跳到异常，中间那句清理永远不会执行——
// 而"登录被拒"恰恰是最容易触发失败的路径，于是每次红一次漏一个账号。
await (async () => {
  const longName = uniq + '_long'
  // 局部部分必须每次唯一：`users.email` 上有唯一约束，固定邮箱意味着
  // "上一次失败残留的账号"会让这一次撞唯一键——而报错是 `duplicate key`，
  // 与被测性质（能不能用长邮箱登录）毫无关系，纯属噪声。
  // 实测踩过：e2e 挂死那次留下的账号就害得同一条 Rust 用例整轮红。
  const longEmail = 'a'.repeat(40) + longName + '@' + 'b'.repeat(20) + '.example.com'
  const longPass = 'Longmail1'
  const emLen = longEmail.length
  if (emLen <= 50) {
    // 掉到 50 或以下的话，整段就测不到"长邮箱"了——宁可直接失败也不要假绿
    throw new Error(`样例邮箱只有 ${emLen} 个字符，长于旧上限的证明已失效`)
  }

  try {
    const reg = await s.apiAs('', 'POST', '/api/auth/register', {
      username: longName,
      email: longEmail,
      password: longPass,
    })
    s.check(`${emLen} 字符邮箱的账号注册成功`, reg.status === 200,
      'status=' + reg.status + ' ' + JSON.stringify(reg.body?.message || ''))

    // **逐字敲进去**，而不是用 harness 的 `setInput` 直接赋值。
    // `setInput` 走原生 value setter，会**绕过 maxlength**——实测上限退回 50 时
    // 它照样能把整个邮箱塞进去，于是"长邮箱能登录"这条断言恒绿。
    // 这里要证明的是**用户真的敲得进去**：敲完后输入框里得有这么多字符。
    await s.typeInto('请输入用户名或邮箱', longEmail)
    const typedLen = await s.evalJs(
      'const el = [...document.querySelectorAll("input")]'
      + '.find(i => (i.placeholder || "").includes("请输入用户名或邮箱"));'
      + ' return el ? el.value.length : -1;',
    )
    s.check(
      `${emLen} 个字符真的敲进了输入框（没有被 maxlength 截断）`,
      typedLen === emLen,
      '敲进去 ' + typedLen + ' / 应为 ' + emLen,
    )

    await fill('请输入密码', longPass)
    await s.clickByText('登 录')
    await waitFor(
      () => s.evalJs('return location.pathname === "/";'),
      { timeout: 20000, label: `用 ${emLen} 字符邮箱登录成功` },
    )
    s.check(`${emLen} 字符的邮箱能通过登录表单登录`, true)
    await s.shot('v018-login-long-email')
  } finally {
    // 先登出（避免带着这个账号的会话去删人），再登 admin 删账号
    const curTok = await s.currentToken().catch(() => null)
    if (curTok) await s.apiAs(curTok, 'POST', '/api/auth/logout').catch(() => {})
    await s.login().catch(() => {})
    const list = await s.api('GET', '/api/admin/users?keyword=' + encodeURIComponent(longName))
    const items = list.body?.data?.items || []
    for (const u of items) {
      if (u.username === longName) await s.api('DELETE', '/api/admin/users/' + u.id)
    }
  }
})()

const failed = s.summary()
await s.stop()
process.exit(failed > 0 ? 1 : 0)
