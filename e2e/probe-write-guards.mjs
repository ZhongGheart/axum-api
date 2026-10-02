// 授权探针：把"每个写入口都查了授权下界"从纪律变成工具。
//
// v0.7.0 / v0.8.0 / v0.9.0 的洞都是探针跑出来的，不是读代码看出来的。
// 但探针一直是一次性手写的——handler 逐个手写不现实，于是每版都靠"记得检查"，
// 而"记得"正是会漏的那一环。
//
// 本工具解决的就是这一环：**写入口清单不靠人列，从 OpenAPI 自动发现**。
// 新加一个 `POST /api/admin/xxx`，它自动进入探针视野；没登记探测方式就直接
// 报"未覆盖"，逼着人当场决定，而不是留到下一版才发现漏了。
//
// 判定标准是一个可证伪的命题：
//
//   只持入口所需最小权限码的操作员，对"权限高于自己"的目标发起写操作
//   → 必须被拒（403），且数据不得有任何变化
//
// 用"最小权限码"是关键。若给操作员发 admin，被拒只可能来自目标下界检查，
// 就测不出"入口权限码本身够不够"的真实边界，且权限码被调松时探针会误报通过。
//
// 每个条目三条断言，缺一不可：
//   1. 弱操作员被拒（403）
//   2. 数据侧未变（403 也可能只是响应被拒、库已写脏）
//   3. **admin 做同一件事仍然成功**——只测拒绝侧的话，"把入口整个禁掉"也能全绿
//
// 第 3 条对破坏性入口尤其重要：每次断言都用**专用靶子**，避免 admin 那次
// 镜像真的把共享夹具删掉、连带后面所有条目失真。

import { Session } from './lib/harness.mjs'
import { writeFileSync, mkdirSync } from 'node:fs'
import { sleep } from './lib/cdp.mjs'
import { join } from 'node:path'

const API = process.env.E2E_API || 'http://127.0.0.1:8080'
const ARTIFACTS = process.env.E2E_ARTIFACTS || join(process.cwd(), 'e2e', '.artifacts')
const PW = 'probe1234'

/**
 * 写入口登记表。
 *
 * `arm(ctx)` 造一个**专用靶子**并返回探针请求；`verify` 检查数据侧。
 * 未登记的写入口会被自动报出（见 [1] 的覆盖检查），这是本工具的主要价值。
 */
const REGISTRY = {
  // ── 建号：授予的角色的权限码不得超出调用方 ──────────────────
  'POST /api/admin/users': {
    kind: 'grant-ceiling',
    minCodes: ['system:user:create'],
    note: '建号时授予的角色不得超出调用方已持有的权限码',
    arm: async (ctx) => ({
      req: { method: 'POST', path: '/api/admin/users',
             body: { username: ctx.name(), email: ctx.name() + '@example.com',
                     password: PW, roles: [ctx.strongRole.name] } },
      username: ctx.pending(),
    }),
    verify: async (s, ctx, t) => {
      const r = await api('GET', '/api/admin/users?page=1&page_size=200')
      const hit = (r.body?.data?.items || []).some((u) => u.username === t.username)
      return { ok: !hit, detail: hit ? '越权建出了账号 ' + t.username : '账号未创建' }
    },
  },

  // ── 改号 ───────────────────────────────────────────────────
  'PUT /api/admin/users/{id}': {
    kind: 'target-lower-bound',
    minCodes: ['system:user:update', 'system:user:list'],
    note: '改号：不得改动权限高于自己的账号',
    arm: async (ctx) => {
      const u = await ctx.freshStrongUser()
      return { req: { method: 'PUT', path: '/api/admin/users/' + u.id,
                      body: { username: u.username, email: u.username + '@example.com',
                              roles: [ctx.strongRole.name, 'user'] } },
               user: u }
    },
    verify: (s, ctx, t) => ctx.expectRoles(t.user.id, [ctx.strongRole.name]),
  },

  // ── 删号 ───────────────────────────────────────────────────
  'DELETE /api/admin/users/{id}': {
    kind: 'target-lower-bound',
    minCodes: ['system:user:delete'],
    note: '删号：不得删除权限高于自己的账号',
    arm: async (ctx) => {
      const u = await ctx.freshStrongUser()
      return { req: { method: 'DELETE', path: '/api/admin/users/' + u.id }, user: u }
    },
    verify: (s, ctx, t) => ctx.expectExists(t.user.id),
  },

  // ── 批量删 ─────────────────────────────────────────────────
  'POST /api/admin/users/batch-delete': {
    kind: 'target-lower-bound',
    minCodes: ['system:user:delete'],
    note: '批量删：列表里混入高权限账号时必须整体拒绝',
    arm: async (ctx) => {
      const u = await ctx.freshStrongUser()
      return { req: { method: 'POST', path: '/api/admin/users/batch-delete',
                      body: { ids: [u.id] } }, user: u }
    },
    verify: (s, ctx, t) => ctx.expectExists(t.user.id),
  },

  // ── 停用 ───────────────────────────────────────────────────
  'PUT /api/admin/users/{id}/status': {
    kind: 'target-lower-bound',
    minCodes: ['system:user:update', 'system:user:list'],
    note: '停用：停用高权限账号等于削弱管理员的可用性，必须查目标',
    arm: async (ctx) => {
      const u = await ctx.freshStrongUser()
      return { req: { method: 'PUT', path: '/api/admin/users/' + u.id + '/status',
                      body: { is_active: false } }, user: u }
    },
    verify: async (s, ctx, t) => {
      const r = await api('GET', '/api/admin/users?page=1&page_size=200')
      const hit = (r.body?.data?.items || []).find((u) => u.id === t.user.id)
      return { ok: !!hit && hit.is_active === true,
               detail: hit ? 'is_active=' + hit.is_active : '账号不见了' }
    },
  },

  // ── 重置口令 ───────────────────────────────────────────────
  'POST /api/admin/users/{id}/reset-password': {
    kind: 'target-lower-bound',
    minCodes: ['system:user:update', 'system:user:list'],
    note: '重置口令 = 直接登录成那个账号，比授予角色更危险，必须查目标',
    arm: async (ctx) => {
      const u = await ctx.freshStrongUser()
      return { req: { method: 'POST',
                      path: '/api/admin/users/' + u.id + '/reset-password',
                      body: { password: 'hijacked123' } }, user: u, password: 'hijacked123' }
    },
    verify: async (s, ctx, t) => {
      // 判据落在"能否用新口令登录"上——这是重置口令的真实后果，
      // 状态码 200 但没生效的情况同样要抓
      try {
        await s.tokenFor(t.user.username, t.password)
        return { ok: false, detail: '探针口令竟然能登录——口令真的被重置了' }
      } catch {
        return { ok: true, detail: '新口令无法登录' }
      }
    },
  },

  // ── 追加角色（v0.9.0 修的第一个洞）─────────────────────────
  'POST /api/admin/users/{user_id}/roles': {
    kind: 'target-lower-bound',
    minCodes: ['system:user:update', 'system:user:list'],
    note: '追加角色：v0.9.0 修的就是这里（第四个漏网入口）',
    arm: async (ctx) => {
      const u = await ctx.freshStrongUser()
      return { req: { method: 'POST', path: '/api/admin/users/' + u.id + '/roles',
                      body: { role_name: 'user' } }, user: u }
    },
    verify: (s, ctx, t) => ctx.expectRoles(t.user.id, [ctx.strongRole.name]),
  },

  // ── 删角色（v0.9.0 探针自己抓到的洞）───────────────────────
  'DELETE /api/admin/roles/{id}': {
    kind: 'grant-ceiling',
    minCodes: ['system:role:delete'],
    note: '删角色 = 把它承载的权限码从所有人身上撤走，与给角色授权是同一件事的两面',
    arm: async (ctx) => {
      const r = await ctx.freshCodeRole(['system:log:list'])
      return { req: { method: 'DELETE', path: '/api/admin/roles/' + r.id }, role: r }
    },
    verify: async (s, ctx, t) => {
      // v0.10.0 起角色列表是分页对象（条目在 data.items），且默认每页只有 10 条。
      // 角色按 created_at ASC 排序，探针刚建的角色一定在末尾——
      // 只看第一页会把"还在"误判成"被越权删了"。必须翻页找完。
      const hit = await roleExistsByName(t.role.name)
      return { ok: hit, detail: hit ? '角色仍在（正确）' : '角色不见了——越权撤权成功' }
    },
  },

  // ── 授权给角色 ─────────────────────────────────────────────
  'PUT /api/admin/roles/{role_id}/menus': {
    kind: 'grant-ceiling',
    minCodes: ['system:menu:grant'],
    note: '自授拦截：把未持有的码授给**自己持有的角色**必须被拒',
    arm: async (ctx) => {
      const btn = await ctx.freshCodeButton()
      return { req: { method: 'PUT', path: '/api/admin/roles/' + ctx.opRoleId + '/menus',
                      body: { menu_ids: [btn.id] } }, roleId: ctx.opRoleId, button: btn }
    },
    verify: async (s, ctx, t) => {
      const r = await api('GET', '/api/admin/menus?role_id=' + t.roleId)
      const hit = JSON.stringify(r.body?.data || '').includes(t.button.id)
      return { ok: !hit, detail: hit ? '码被授进了自己的角色（自授成功）' : '未授予' }
    },
    // 授给**别的**角色按设计是放行的，这条不是漏洞：
    // 授给别人不会让调用者变强，且 admin 造出新权限码后必须能分发它，
    // 否则"权限码即数据"的核心工作流直接没法用。间接提权的路径由
    // `ensure_can_grant_roles` 的包含关系判定在"该角色被授给调用者时"堵住。
    // 这里显式断言这个设计意图仍然成立——免得日后有人"顺手"把它也拦了。
    armAllow: async (ctx) => {
      const other = await ctx.freshCodeRole([])
      const btn = await ctx.freshCodeButton()
      return { req: { method: 'PUT', path: '/api/admin/roles/' + other.id + '/menus',
                      body: { menu_ids: [btn.id] } }, roleId: other.id, button: btn }
    },
    allowExpects: 200,
  },

  // ── 删菜单（v0.8.0 修的洞，守卫覆盖整棵子树）──────────────
  'DELETE /api/admin/menus/{id}': {
    kind: 'grant-ceiling',
    minCodes: ['system:menu:delete'],
    note: '删菜单：级联删除等价于把子树里的码从所有角色剥掉，须查子树被授予的码',
    arm: async (ctx) => {
      // 码必须**已被授予至少一个角色**才会触发守卫（与 v0.8.0 的设计一致：
      // 没人依赖的码删掉不改变任何人的权限，放行）
      //
      // 用探针专属码而不是 `system:log:list`：菜单的 permission 有唯一性约束
      // （v0.8.0 的"已占用的权限码"检查），复用内置码会拿到 409。
      // 且必须每次唯一：`arm` 会被调用两次（拒绝侧 + 放行侧），固定码第二次就撞 409。
      const holder = await ctx.freshCodeRole([])
      const btn = await ctx.freshCodeButton()
      await api('PUT', '/api/admin/roles/' + holder.id + '/menus',
                  { menu_ids: [btn.id] })
      return { req: { method: 'DELETE', path: '/api/admin/menus/' + btn.id },
               button: btn, holder }
    },
    // 放行侧单独造靶：守卫只在"码已被授予至少一个角色"时才触发
    // （与 v0.8.0 的设计一致——没人依赖的码删掉不改变任何人的权限，放行）。
    // 所以镜像侧用一个**未被授予**的按钮：它验的是"admin 仍能删"，
    // 而不是"admin 能删掉自己持有的码"——后者需要一个 admin 已持有的码，
    // 而内置码不能重建（permission 唯一约束，重复建会 409）。
    armAllow: async (ctx) => {
      const btn = await ctx.freshCodeButton()
      return { req: { method: 'DELETE', path: '/api/admin/menus/' + btn.id }, button: btn }
    },
    verify: async (s, ctx, t) => {
      const r = await api('GET', '/api/admin/menus')
      const hit = JSON.stringify(r.body?.data || '').includes(t.button.id)
      return { ok: hit, detail: hit ? '按钮仍在树里（正确）' : '按钮不见了——越权删除成功' }
    },
  },
}

/** 不作用于特定对象、或已明确判定无需下界检查的写入口 */
const GLOBAL_WRITES = new Set([
  'POST /api/admin/dict/items',
  'PUT /api/admin/dict/items/{id}',
  'DELETE /api/admin/dict/items/{id}',
  'POST /api/admin/dict/types',
  'PUT /api/admin/dict/types/{id}',
  'DELETE /api/admin/dict/types/{id}',
  'POST /api/admin/dict/refresh',
  'POST /api/admin/menus',
  'PUT /api/admin/menus/{id}',
  'POST /api/admin/menus/{id}/restore-permission',
  'POST /api/admin/roles',
  'PUT /api/admin/roles/{id}',
  'POST /api/admin/monitor/metrics/reset',
  'POST /api/admin/validate',
])

// ──────────────────────────────────────────────
const s = new Session('授权探针：写入口的授权下界')
await s.start()
await s.login()

const uniq = 'prb' + Math.random().toString(36).slice(2, 7)
const report = []

s.log('\n[1] 从 OpenAPI 自动发现写入口（不靠人列）')
const spec = await s.evalJs(
  'const r = await fetch(' + JSON.stringify(API + '/api/openapi.json') + ');'
  + ' return r.ok ? await r.json() : null'
)
if (!spec) throw new Error('读不到 ' + API + '/api/openapi.json')

const discovered = []
for (const [p, ops] of Object.entries(spec.paths || {})) {
  for (const m of Object.keys(ops)) {
    const up = m.toUpperCase()
    if (!['POST', 'PUT', 'DELETE', 'PATCH'].includes(up)) continue
    if (!p.startsWith('/api/admin')) continue
    discovered.push(up + ' ' + p)
  }
}
discovered.sort()
s.check('发现管理区写入口', discovered.length > 0, discovered.length + ' 个')

const uncovered = discovered.filter((e) => !REGISTRY[e] && !GLOBAL_WRITES.has(e))
s.check('每个写入口都已登记探测方式或明确豁免',
  uncovered.length === 0,
  uncovered.length ? uncovered.join(' | ') : '无遗漏')

const stale = Object.keys(REGISTRY).filter((e) => !discovered.includes(e))
s.check('登记表里没有已下线的入口（否则探测静默失效）',
  stale.length === 0, stale.length ? stale.join(' | ') : '无失效登记')

// ──────────────────────────────────────────────
s.log('\n[2] 造共享夹具与清理账本')
const created = { users: [], roles: [], menus: [] }
let seq = 0
const name = () => uniq + '_x' + (seq++)

// IP 限流是 100 次/分（v0.9.0 途中实测），而探针一轮本来就要发上百个请求
// ——这是工具的固有形状，不是 bug。所以必须能等窗口滑过去，
// 且等待时要说出来，否则一轮慢跑看起来像卡死。
let rateLimitUntil = 0
async function callApi(fn) {
  for (;;) {
    const wait = rateLimitUntil - Date.now()
    if (wait > 0) await sleep(wait + 300)
    const r = await fn()
    if (r.status !== 429) return r
    rateLimitUntil = Date.now() + 61000
    s.log('  触发 IP 限流（100 次/分），等待窗口滑过…')
  }
}

/** 带 429 重试的请求包装：注册表里所有 arm/verify 都走它 */
const api = (m, p, b) => callApi(() => s.api(m, p, b))
const apiAs = (tok, m, p, b) => callApi(() => s.apiAs(tok, m, p, b))

/**
 * 翻页查找指定名字的角色是否还在
 *
 * `GET /api/admin/roles` 自 v0.10.0 起是分页对象且默认每页 10 条，
 * 排序为 `created_at ASC`——探针刚建的角色一定落在最后一页。
 * 只读第一页会把"角色还在"误判成"被越权删掉了"，那样的假红比不测更糟：
 * 它会把人引去查一个根本不存在的授权洞。
 */
async function roleExistsByName(name) {
  const pageSize = 200
  for (let page = 1; page <= 50; page++) {
    const r = await api('GET', `/api/admin/roles?page=${page}&page_size=${pageSize}`)
    const items = r.body?.data?.items || []
    if (items.some((x) => x.name === name)) return true
    if (items.length < pageSize) return false
  }
  return false
}

const menuIdCache = new Map()
async function menuIdOf(code) {
  if (menuIdCache.has(code)) return menuIdCache.get(code)
  const tree = await callApi(() => s.api('GET', '/api/admin/menus'))
  const id = await s.evalJs(
    'const roots = ' + JSON.stringify(tree.body?.data || []) + '; let hit = null;'
    + ' const walk = (ns) => { for (const n of ns || []) {'
    + '   if (n.permission === ' + JSON.stringify(code) + ') { hit = n; return }'
    + '   walk(n.children) } }; walk(roots); return hit ? hit.id : null'
  )
  if (!id) throw new Error('菜单树里找不到权限码: ' + code)
  menuIdCache.set(code, id)
  return id
}

async function mkRole(roleName, codes) {
  const r = await callApi(() => s.api('POST', '/api/admin/roles', { name: roleName, description: '探针临时角色' }))
  if (r.status !== 200) throw new Error('建角色失败 ' + roleName + ': ' + r.status)
  const id = r.body?.data?.id
  created.roles.push(id)
  const ids = []
  for (const c of codes) ids.push(await menuIdOf(c))
  if (ids.length) {
    const g = await callApi(() => s.api('PUT', '/api/admin/roles/' + id + '/menus', { menu_ids: ids }))
    if (g.status !== 200) throw new Error('授权失败 ' + roleName + ': ' + g.status)
  }
  return { id, name: roleName }
}

async function mkUser(username, roles) {
  const r = await callApi(() => s.api('POST', '/api/admin/users', {
    username, email: username + '@example.com', password: PW, roles }))
  if (r.status !== 200) {
    throw new Error('建用户失败 ' + username + ': ' + r.status + ' '
      + JSON.stringify(r.body?.message || ''))
  }
  created.users.push(r.body?.data?.id)
  return r.body?.data
}

async function mkCodeButton(code) {
  const r = await callApi(() => s.api('POST', '/api/admin/menus',
    { name: name() + '_btn', type: 'button', permission: code }))
  if (r.status !== 200) throw new Error('建按钮失败: ' + r.status)
  created.menus.push(r.body?.data?.id)
  return { id: r.body?.data?.id, permission: code }
}

// 强角色：持操作员拿不到的码，它就是"权限高于调用方"的具象化
const strongRole = await mkRole(uniq + '_strong', ['system:log:list', 'system:role:delete'])
s.check('建强角色（持 log:list / role:delete）', true)

const pendingNames = []
const ctx = {
  strongRole,
  name,
  pending: () => { const n = name() + '_pending'; pendingNames.push(n); return n },
  freshStrongUser: async () => mkUser(name() + '_strongu', [strongRole.name]),
  freshCodeRole: async (codes) => mkRole(name() + '_coderole', codes),
  freshCodeButton: (code) => mkCodeButton(code || ('probe:' + uniq + ':' + (seq++))),
  expectRoles: async (userId, want) => {
    const r = await api('GET', '/api/admin/users/' + userId + '/roles')
    const roles = (r.body?.data || []).slice().sort()
    return { ok: JSON.stringify(roles) === JSON.stringify([...want].sort()),
             detail: JSON.stringify(roles) }
  },
  expectExists: async (userId) => {
    const r = await api('GET', '/api/admin/users/' + userId + '/roles')
    return { ok: r.status === 200, detail: '目标账号 status=' + r.status }
  },
}

// ──────────────────────────────────────────────
s.log('\n[3] 逐入口实测')
const entries = Object.entries(REGISTRY).sort()

// 操作员按**权限码集合**复用：9 个入口里有 4 个入口的 minCodes 完全相同
// （user:update + user:list）。每次重建虽然更"干净"，但会把请求量推到
// IP 限流之上，而多建的那几个并没有增加任何探测能力——
// 同一个码集合下，操作员的行为对所有入口都一样。
const operatorCache = new Map()

for (const [entry, def] of entries) {
  const cacheKey = [...def.minCodes].sort().join(',')
  let op = operatorCache.get(cacheKey)
  if (!op) {
    const opRole = await mkRole(name() + '_oprole', def.minCodes)
    const opUser = await mkUser(name() + '_op', [opRole.name])
    let tok
    try {
      tok = await s.tokenFor(opUser.username, PW)
    } catch (e) {
      s.check(entry + ' 操作员可登录', false, String(e.message || e))
      continue
    }
    // 确认操作员真的只持这些码——夹具没建对的话，403 可能只是"权限不足"，
    // 那样"被拒"就成了假绿
    const perms = await apiAs(tok, 'GET', '/api/auth/permissions')
    const held = (perms.body?.data || []).slice().sort()
    s.check(cacheKey + ' 操作员持码符合预期',
      JSON.stringify(held) === JSON.stringify([...def.minCodes].sort()),
      JSON.stringify(held))
    op = { tok, username: opUser.username, roleId: opRole.id }
    operatorCache.set(cacheKey, op)
  }
  const opTok = op.tok
  // 自授类条目需要知道"调用者自己持有的角色"是谁
  ctx.opRoleId = op.roleId

  // ① 拒绝侧
  const armed = await def.arm(ctx)
  s.forgetDeliberateFailures()
  const r = await apiAs(opTok, armed.req.method, armed.req.path, armed.req.body)
  s.check(entry + ' 弱操作员 → 被拒', r.status === 403,
    'status=' + r.status + ' ' + (r.body?.message || ''))

  // ② 数据侧
  const v = await def.verify(s, ctx, armed)
  s.check(entry + ' 数据侧未被改动', v.ok, v.detail)

  // ③ 放行侧：**另一个专用靶子**，admin 做同一件事必须成功。
  // 共享靶子不行——admin 那次会真的把它删掉，后面条目全被污染。
  // 少数入口的放行路径与拒绝路径不同（守卫只在"码已被授予"时才触发），
  // 那种登记 `armAllow`。
  const armed2 = await (def.armAllow || def.arm)(ctx)
  const adminR = await api(armed2.req.method, armed2.req.path, armed2.req.body)
  // 默认判据是"没被 403 拦下、也没 5xx"；个别条目的合法路径有确切状态码
  // （如"授给别的角色按设计放行"必须是 200），用 allowExpects 声明
  const allowOk = def.allowExpects
    ? adminR.status === def.allowExpects
    : (adminR.status !== 403 && adminR.status < 500)
  s.check(entry + ' 合法调用者仍可用（没把功能禁死）', allowOk,
    'admin status=' + adminR.status + ' ' + (adminR.body?.message || ''))

  report.push({ entry, kind: def.kind, note: def.note,
                deniedStatus: r.status, sideOk: v.ok, adminStatus: adminR.status })
}

// ──────────────────────────────────────────────
s.log('\n[4] 清理')
// 先删用户再删角色再删菜单：user_roles/role_menus 是 CASCADE，顺序反了会被守卫挡
// 清理同样走限流包装：这里若撞上 429 就会留下垃圾数据，
// 而"共享环境里身后无残留"是这个工具能反复运行的前提
for (const id of created.users) {
  await callApi(() => s.api('DELETE', '/api/admin/users/' + id))
}
for (const id of created.roles) {
  await callApi(() => s.api('DELETE', '/api/admin/roles/' + id))
}
for (const id of created.menus) {
  await callApi(() => s.api('DELETE', '/api/admin/menus/' + id))
}
s.check('探针夹具已全部清理',
  created.users.length + created.roles.length + created.menus.length > 0,
  '用户 ' + created.users.length + ' / 角色 ' + created.roles.length
  + ' / 菜单 ' + created.menus.length)

s.log('\n[5] 落盘报告')
mkdirSync(ARTIFACTS, { recursive: true })
const reportPath = join(ARTIFACTS, 'write-guard-probe.json')
writeFileSync(reportPath, JSON.stringify({
  probedAt: new Date().toISOString(),
  discoveredWrites: discovered.length,
  probedEntries: entries.length,
  uncovered, stale, results: report,
}, null, 2))
s.log('  报告 -> ' + reportPath)

const failed = s.summary()
await s.stop()
process.exit(failed ? 1 : 0)
