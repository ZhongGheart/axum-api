// 套件 3：角色追加的授权下界与会话吊销（v0.9.0）
//
// `POST /api/admin/users/:id/roles` 是**追加**语义（不覆盖既有角色），
// 所以用户表单那道"整体替换"守卫覆盖不到它。v0.5.0 在这里补过一次
// "授予什么"的边界，v0.9.0 补上的是"授予给谁"——三处一起：
//
//   1. 只持 `system:user:update` 的角色曾能给纯 admin 账号追加角色（实测 200）
//   2. 追加角色后不吊销存量会话，新权限要等目标重新登录才生效
//   3. 目标用户不存在时外键违例冒成 500，而非 404
//
// 判定标准落在**可观测事实**上：目标账号的角色集合真的没变、
// 旧令牌真的 401 了、404 而不是 500。状态码 200 也可能什么都没写，
// 只断言状态码的话，"把追加整个禁掉"也能全绿——所以每条拒绝侧断言
// 都配一条数据侧断言；同时每条拒绝侧都配一条放行侧（[2]/[3]/[6]）。
//
// 跨身份是这套的核心：管理员建号 → 弱操作员尝试越权 → 目标用户验会话。
// 因此令牌用 `apiAs`/`tokenFor` 显式传递，不去改浏览器里 admin 的会话。

import { Session } from '../lib/harness.mjs'

const s = new Session('角色追加授权下界')
await s.start()

const uniq = 'v09' + Math.random().toString(36).slice(2, 8)
const PW = 'user1234'

// 管理员建出来的账号带"强制改密"标记（v0.11.0），拿到的是受限令牌，
// 打业务接口只会得到"请先修改初始密码"的 403。因此建号后要先让该用户
// 自助改一次密，下面 PW2 就是改完之后在用的口令。
const PW2 = 'user5678'

/** 按权限码在菜单树里找出对应节点 id（授权接口吃的是 menu_id，不是权限码） */
async function menuIdOf(code) {
  const tree = await s.api('GET', '/api/admin/menus')
  const found = await s.evalJs(
    'const roots = ' + JSON.stringify(tree.body?.data || []) + ';'
    + ' let hit = null;'
    + ' const walk = (ns) => { for (const n of ns || []) {'
    + '   if (n.permission === ' + JSON.stringify(code) + ') { hit = n; return }'
    + '   walk(n.children) } };'
    + ' walk(roots);'
    + ' return hit ? hit.id : null'
  )
  if (!found) throw new Error('菜单树里找不到权限码: ' + code)
  return found
}

async function createRole(name, codes) {
  const r = await s.api('POST', '/api/admin/roles', { name, description: 'e2e 临时角色' })
  if (r.status !== 200) throw new Error('新建角色 ' + name + ' 失败: ' + r.status)
  const id = r.body?.data?.id
  const ids = []
  for (const c of codes) ids.push(await menuIdOf(c))
  const g = await s.api('PUT', '/api/admin/roles/' + id + '/menus', { menu_ids: ids })
  if (g.status !== 200) throw new Error('给角色 ' + name + ' 授权失败: ' + g.status)
  return { id, name }
}

async function createUser(username, roles) {
  const r = await s.api('POST', '/api/admin/users', {
    username,
    email: username + '@example.com',
    password: PW,
    roles,
  })
  if (r.status !== 200) {
    throw new Error('创建用户 ' + username + ' 失败: ' + r.status + ' '
      + JSON.stringify(r.body?.message || ''))
  }
  return r.body?.data?.id
}

async function rolesOf(userId) {
  const r = await s.api('GET', '/api/admin/users/' + userId + '/roles')
  return (r.body?.data || []).slice().sort()
}

async function dropUser(userId) {
  await s.api('DELETE', '/api/admin/users/' + userId)
}
async function dropRole(roleId) {
  await s.api('DELETE', '/api/admin/roles/' + roleId)
}

// ──────────────────────────────────────────────
s.log('\n[1] 真实登录 admin 并造齐三个角色')
await s.login()
s.check('登录 admin', true)

const perm = {
  USER_LIST: 'system:user:list',
  USER_UPDATE: 'system:user:update',
  DICT_LIST: 'system:dict:list',
  ROLE_DELETE: 'system:role:delete',
}

// 弱操作员：只有 user:list + user:update，够触发守卫但权限低于目标
const weak = await createRole(uniq + '_weak', [perm.USER_LIST, perm.USER_UPDATE])
s.check('建弱操作员角色（仅 user:list + user:update）', true)

// 目标账号的角色：持一个操作员拿不到的码（role:delete），权限明确更高
const strong = await createRole(uniq + '_strong', [perm.ROLE_DELETE])
s.check('建高权限目标角色（role:delete）', true)

// 用来验证会话吊销的授予角色：恰好含 dict:list
const granted = await createRole(uniq + '_granted', [perm.DICT_LIST])
s.check('建授予角色（dict:list）', true)

const weakUser = uniq + '_operator'
const weakId = await createUser(weakUser, [weak.name])
s.check('建弱操作员账号', !!weakId, 'id=' + weakId)

const strongUser = uniq + '_boss'
const strongId = await createUser(strongUser, [strong.name])
s.check('建高权限目标账号', !!strongId, 'id=' + strongId)

const lazyUser = uniq + '_lazy'
const lazyId = await createUser(lazyUser, ['user'])
s.check('建会话验证目标账号（仅内置 user 角色）', !!lazyId, 'id=' + lazyId)

s.resetBadResponses()

// ──────────────────────────────────────────────
s.log('\n[2] 洞 A：不得给权限高于自己的账号追加角色')
const weakTok = await s.activatedToken(weakUser, PW, PW2)
const before = await rolesOf(strongId)
s.check('目标账号当前只持强角色',
  before.length === 1 && before[0] === strong.name, JSON.stringify(before))

const denied = await s.apiAs(weakTok, 'POST',
  '/api/admin/users/' + strongId + '/roles', { role_name: 'user' })
s.check('弱操作员给高权限账号追加角色 → 被拒', denied.status === 403,
  'status=' + denied.status)
s.log('  界面文案: ' + (denied.body?.message || '(无)'))

// 判据落在数据上：被拒之后角色集合必须原样不动
const after = await rolesOf(strongId)
s.check('被拒后目标账号角色集合原样不变',
  JSON.stringify(after) === JSON.stringify(before),
  'before=' + JSON.stringify(before) + ' after=' + JSON.stringify(after))

// ──────────────────────────────────────────────
s.log('\n[3] 洞 A 的镜像：合法的自我调整不被误伤')
// 弱操作员给自己追加一个自己已持有的弱角色是合法的，不是越权：
// 这条守卫对自己天然恒真，修复不该把这条路一起堵掉。
const selfRolesBefore = await rolesOf(weakId)
const selfAppend = await s.apiAs(weakTok, 'POST',
  '/api/admin/users/' + weakId + '/roles', { role_name: 'user' })
s.check('弱操作员给自己追加弱角色 → 放行', selfAppend.status === 200,
  'status=' + selfAppend.status)
const selfRolesAfter = await rolesOf(weakId)
s.check('自我追加真的落库且未覆盖原角色',
  selfRolesAfter.includes('user') && selfRolesAfter.length === selfRolesBefore.length + 1,
  'before=' + JSON.stringify(selfRolesBefore) + ' after=' + JSON.stringify(selfRolesAfter))

// ──────────────────────────────────────────────
s.log('\n[4] 洞 B：追加角色后旧令牌必须立刻失效')
// 先证明这个账号现在确实读不到字典——否则后面"200"说明不了任何事
const lazyTok = await s.activatedToken(lazyUser, PW, PW2)
const pre = await s.apiAs(lazyTok, 'GET', '/api/admin/dict/types')
s.check('授权前目标账号读字典 → 403', pre.status === 403, 'status=' + pre.status)

const assigned = await s.api('POST',
  '/api/admin/users/' + lazyId + '/roles', { role_name: granted.name })
s.check('admin 给目标账号追加角色', assigned.status === 200,
  'status=' + assigned.status + ' ' + (assigned.body?.message || ''))

// 关键：**不重新登录**，继续用原来那个令牌。
// 判别点是 403 → 401：不是让它"拿旧令牌用新权限"，而是旧令牌立刻作废。
const stale = await s.apiAs(lazyTok, 'GET', '/api/admin/dict/types')
s.check('追加后旧令牌立刻失效（会话已吊销）', stale.status === 401,
  'status=' + stale.status + ' ' + (stale.body?.message || ''))
s.check('失效形态是 401 而不是 403（否则等于没吊销）',
  stale.status !== 403, 'status=' + stale.status)

// 这个 401 是**故意**造出来的，清掉记录，让 401 在余下阶段仍是真信号。
// 否则只能把它加进网络白名单——那样本套件任何一处意外 401（令牌过期、
// 会话被误吊销）都会被当成"预期"，恰好是它要抓的那类回归。
s.forgetDeliberateFailures()

const freshTok = await s.tokenFor(lazyUser, PW2)
const relogin = await s.apiAs(freshTok, 'GET', '/api/admin/dict/types')
s.check('重新登录后新权限可用', relogin.status === 200,
  'status=' + relogin.status + ' ' + (relogin.body?.message || ''))

// ──────────────────────────────────────────────
s.log('\n[5] 幂等：重复追加同一角色不该踢人下线')
// 追加用的是 ON CONFLICT DO NOTHING，第二次什么都没写。
// 若照样吊销会话，运维点两下保存就把对方踢下线了——那比不吊销更糟。
const reapply = await s.api('POST',
  '/api/admin/users/' + lazyId + '/roles', { role_name: granted.name })
s.check('重复追加同一角色返回 200', reapply.status === 200, 'status=' + reapply.status)
const stillAlive = await s.apiAs(freshTok, 'GET', '/api/admin/dict/types')
s.check('重复追加后目标会话仍然有效', stillAlive.status === 200,
  'status=' + stillAlive.status + ' ' + (stillAlive.body?.message || ''))

// ──────────────────────────────────────────────
s.log('\n[6] 第三个洞：目标用户不存在应报 404')
const ghost = await s.api('POST',
  '/api/admin/users/00000000-0000-0000-0000-000000000000/roles',
  { role_name: 'user' })
s.check('给不存在的用户追加角色 → 404', ghost.status === 404,
  'status=' + ghost.status + ' ' + (ghost.body?.message || ''))
s.check('不是 500（入参错误不得冒成服务端故障）', ghost.status !== 500,
  'status=' + ghost.status)

// ──────────────────────────────────────────────
s.log('\n[7] 用户管理页真实渲染')
await s.goto('/system/user')
await s.evalJs('return new Promise(r => setTimeout(r, 1200))')
s.check('用户管理页渲染正常',
  (await s.evalJs('return document.body.innerText.includes("用户")')) === true)
await s.shot('v09-role-assign-guard')

// 清理：共享环境里每个用例都应身后无残留
await dropUser(lazyId)
await dropUser(strongId)
await dropUser(weakId)
await dropRole(granted.id)
await dropRole(strong.id)
await dropRole(weak.id)

// 本套件预期的 4xx，逐个都有出处：
//   403 —— 洞 A 的越权拒绝，以及授权前目标账号读字典被拒
//   404 —— 第三个洞，给不存在的用户追加角色
// [4] 里那个故意造出来的 401 已就地 `resetBadResponses`，不靠白名单放行。
// 同一份清单同时用于网络与控制台两处检查，避免两处声明漂移。
const EXPECTED_HTTP = ['403 ', '404 ']
s.checkNoConsoleErrors(EXPECTED_HTTP)
s.checkNoUnexpectedHttp('除预期的 403/404 外无 4xx/5xx', EXPECTED_HTTP)

const failed = s.summary()
await s.stop()
process.exit(failed ? 1 : 0)
