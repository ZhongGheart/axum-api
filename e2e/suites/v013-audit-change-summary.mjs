// v0.13.0 界面自查：审计要能回答"改了什么"
//
// 这一版的整条主线是"记下来的东西得读得到"。集成测试证明的是**库里**有摘要；
// 这里证明的是**界面表格与导出的 xlsx 里**也读得到。一列加了但取错字段
//（比如取 `params` 而不是 `result`），接口测试照样绿——只有把界面和文件
// 打开才看得见，所以这两层必须单独验。

import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { Session, waitFor, sleep, ARTIFACTS } from '../lib/harness.mjs'

const s = new Session('v0.13.0 审计变更摘要')
const DOWNLOADS = join(ARTIFACTS, 'downloads')

await s.start()

const uniq = 'v013' + Math.random().toString(36).slice(2, 8)
const wait = (ms) => sleep(ms)

// 轮询审计接口，等某一行的 result 落库。
// 审计是异步写的：刚做完写操作立刻查会读到空，那种红是竞态而不是缺陷。
async function pollResult(match) {
  for (let i = 0; i < 40; i++) {
    const r = await s.api('GET', '/api/admin/audit-logs?page_size=100')
    const rows = r.body?.data?.items || []
    const hit = rows.find(match)
    if (hit) return hit
    await wait(100)
  }
  return null
}

s.log('\n[1] 真实登录 admin')
await s.login()
s.check('登录 admin', true)

// ── 造几条带摘要的写操作 ────────────────────────────────────

s.log('\n[2] 造带摘要的写操作')

const created = await s.api('POST', '/api/admin/roles', {
  name: uniq + '_role',
  description: 'v0.13.0 摘要断言的来源',
})
const roleId = created.body?.data?.id
s.check('临时角色已创建', created.status === 200 && !!roleId,
  'status=' + created.status + ' id=' + roleId)

// 角色重名时，光有"新建角色"四个字答不出"是哪一个"
const createRow = await pollResult((x) =>
  x.method === 'POST' && x.path === '/api/admin/roles'
  && String(x.result || '').includes(uniq + '_role'))
s.check('建角色的审计行带上了摘要', !!createRow,
  createRow ? createRow.result : '轮询 4s 未等到带摘要的行')
s.check('摘要里含资源名（能认出是哪个角色）',
  !!createRow && String(createRow.result).includes(uniq + '_role'),
  createRow ? createRow.result : '')

if (roleId) {
  const renamed = await s.api('PUT', '/api/admin/roles/' + roleId, {
    name: uniq + '_renamed',
    description: 'v0.13.0 摘要断言的来源',
  })
  s.check('临时角色已改名', renamed.status === 200, 'status=' + renamed.status)
  // 改名要**两个名字都记**：事后只看到新名，答不出"原来叫什么"
  const updateRow = await pollResult((x) =>
    x.method === 'PUT' && x.path === '/api/admin/roles/' + roleId
    && String(x.result || '').includes(uniq + '_renamed'))
  s.check('改名的审计行带上了摘要', !!updateRow,
    updateRow ? updateRow.result : '轮询 4s 未等到带摘要的行')
  s.check('改名摘要同时含旧名与新名',
    !!updateRow && String(updateRow.result).includes(uniq + '_role')
    && String(updateRow.result).includes(uniq + '_renamed'),
    updateRow ? updateRow.result : '')
}

// ── 口令重置：摘要要说明白重置了谁的，但不能带出口令本身 ────

// roles 必须给：没有角色的用户登录后没有任何权限，后端直接 400 拒建
const pwd = await s.api('POST', '/api/admin/users', {
  username: uniq + '_u',
  email: uniq + '_u@example.com',
  password: 'Str0ng!' + uniq + 'Pw',
  roles: ['user'],
})
const pwdUserId = pwd.body?.data?.id
s.check('临时用户已创建', pwd.status === 200 && !!pwdUserId, 'status=' + pwd.status)

const SECRET = 'Reset!' + uniq + '2026'
if (pwdUserId) {
  const rst = await s.api('POST',
    '/api/admin/users/' + pwdUserId + '/reset-password', { password: SECRET })
  s.check('口令已重置', rst.status === 200, 'status=' + rst.status)
  const pwdRow = await pollResult((x) =>
    String(x.path || '').includes('/reset-password')
    && String(x.result || '').includes(uniq + '_u'))
  s.check('重置口令的审计行带上了摘要', !!pwdRow,
    pwdRow ? pwdRow.result : '轮询 4s 未等到带摘要的行')
  s.check('口令摘要点名了被重置的人', !!pwdRow && String(pwdRow.result).includes(uniq + '_u'),
    pwdRow ? pwdRow.result : '')
}

// ── 界面表格读得到 ──────────────────────────────────────────

s.log('\n[3] 系统日志页：变更摘要列')
await s.goto('/system/log', 1200)

const cols = await s.evalJs(
  'return [...document.querySelectorAll(".n-data-table-thead th")]'
  + '.map(th => th.innerText.trim()).filter(Boolean)')
s.check('表头里有「变更摘要」列', cols.includes('变更摘要'), JSON.stringify(cols))

// 筛出刚才那批写操作，逐格核对摘要确实显示出了内容。
// 只判"列在不在"太弱：一个把 result 取成空串的实现同样能过。
await s.setInput('操作', '/api/admin/roles')
await s.clickByText('查询')
await wait(1200)

const cells = await s.evalJs(
  'const rows = [...document.querySelectorAll(".n-data-table-tbody .n-data-table-tr")];'
  + ' const out = [];'
  + ' for (const tr of rows) {'
  + '   const tds = [...tr.querySelectorAll("td")].map(td => td.innerText.trim());'
  + '   if (tds.length) out.push(tds);'
  + ' }'
  + ' return out;')
const nameOnScreen = cells.some((tds) => tds.some((t) => t.includes(uniq + '_role')
  || t.includes(uniq + '_renamed')))
s.check('界面上看得见那条摘要', nameOnScreen,
  cells.length ? JSON.stringify(cells[0]) : '筛完没有数据行')

// 破折号只该出现在**纯读操作**上：读操作没有"改了什么"可写，
// 显示破折号正是为了区分"本来就没内容"与"没记"。
// 所以这里不能笼统地要求整列没有破折号——筛选串 `/api/admin/roles`
// 同时命中 GET 读操作，那些行的破折号是**正确**的。
// 要卡的是另一头：写操作一旦也是破折号，说明这一列压根没接上。
const DASH = ['\u2014', '-', '']
const summaryCol = cells.length ? cells[0].length - 1 : -1
const writes = cells.filter((tds) => tds[2] && tds[2] !== 'GET')
const writesDashed = writes.filter((tds) => DASH.includes(tds[summaryCol]))
s.check('写操作的摘要列不是破折号', cells.length > 0 && writes.length > 0
  && writesDashed.length === 0,
  '总行=' + cells.length + ' 写操作=' + writes.length
  + ' 写操作里是破折号=' + writesDashed.length + ' 列序号=' + summaryCol)
await s.shot('v013-log-change-summary')

// 复位筛选：导出断言要在全量上做，不能跑在筛过的子集上
await s.clickByText('重置')
await wait(1000)

// ── 导出的 xlsx 里也有这一列 ────────────────────────────────

s.log('\n[4] 导出 Excel 含变更摘要列')
// 前端把导出文件名写死成「操作日志.xlsx」，Chrome 遇到同名文件是**覆盖**
// 而不是去重加「(1)」后缀。所以"目录里多出一个新文件"这种判据永远不成立——
// 上一次跑留下的同名文件会把判据永久性地钉死。
// 改看修改时间：只认本次点击之后被写过的 xlsx。
const clickedAt = Date.now()
await s.resetBadResponses()
await s.clickByText('导出 Excel')

const freshXlsx = () => {
  for (const f of readdirSync(DOWNLOADS)) {
    if (!f.endsWith('.xlsx') || f.endsWith('.crdownload')) continue
    const st = statSync(join(DOWNLOADS, f))
    if (st.mtimeMs >= clickedAt - 1000) return f
  }
  return false
}
const exported = await waitFor(() => freshXlsx() || false,
  { timeout: 20000, label: 'xlsx 落盘' }).catch(() => null)
s.check('导出文件已落盘', !!exported, exported || '未找到本次写出的 .xlsx')

if (exported) {
  const path = join(DOWNLOADS, exported)
  const magic = readFileSync(path).subarray(0, 2).toString('latin1')
  const unzip = (inner) => spawnSync('unzip', ['-p', path, inner]).stdout.toString('utf8')
  const strings = unzip('xl/sharedStrings.xml')
  const sheet = unzip('xl/worksheets/sheet1.xml')
  s.check('内容是合法 xlsx（而非错误页）', magic === 'PK', 'magic=' + magic)
  s.check('xlsx 里有「变更摘要」表头', strings.includes('变更摘要'))
  // 表头在而内容空，一个把 result 取错的实现也能骗过上一条
  const inXlsx = strings.includes(uniq + '_role')
  s.check('xlsx 数据行里能看到摘要内容', inXlsx,
    inXlsx ? 'sharedStrings 命中 ' + uniq + '_role' : 'sharedStrings 里没找到 ' + uniq + '_role')
  const rowsInSheet = (sheet.match(/<row /g) || []).length
  s.check('xlsx 不止表头一行', rowsInSheet > 1, 'row 元素 ' + rowsInSheet + ' 个')
}
s.checkNoUnexpectedHttp('导出过程无 4xx/5xx')

// ── 口令一个字都不入库 ──────────────────────────────────────

s.log('\n[5] 库里没有明文口令')
const dump = await s.api('GET', '/api/admin/audit-logs?page_size=100')
const joined = (dump.body?.data?.items || [])
  .map((x) => (x.params || '') + ' ' + (x.result || '')).join('\n')
// params 与 result 都要扫：params 是查询串，只查 result 的话，
// 有人改成"把请求体记进 params"时秘密就从另一列漏出去了
s.check('params 与 result 两列都不含明文口令',
  !joined.includes(SECRET) && !joined.includes('admin123'),
  joined.includes(SECRET) || joined.includes('admin123')
    ? '查到了明文口令' : '未发现')

// ── 无残留 ──────────────────────────────────────────────────

s.log('\n[6] 无残留')
if (roleId) {
  const del = await s.api('DELETE', '/api/admin/roles/' + roleId)
  s.check('临时角色已删除', del.status === 200, 'status=' + del.status)
}
if (pwdUserId) {
  const delU = await s.api('DELETE', '/api/admin/users/' + pwdUserId)
  s.check('临时用户已删除', delU.status === 200, 'status=' + delU.status)
}

const left = await s.api('GET', '/api/admin/roles?page_size=200')
const leftRoles = (left.body?.data?.items || [])
  .filter((x) => x.name.startsWith(uniq)).length
s.check('临时角色没有残留', leftRoles === 0, '残留 ' + leftRoles + ' 个')

s.checkNoConsoleErrors([])
const failed = s.summary()
await s.stop()
process.exit(failed > 0 ? 1 : 0)
