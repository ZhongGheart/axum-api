// v0.10.0 界面自查：新增的筛选控件与分页必须**真的**生效
//
// 这一版的整条主线是"界面不许说谎"。前面的集成测试证明的是
// **接口**筛得对；这里证明的是**界面把参数发出去了**——
// 控件摆着但没接上事件，是同一族问题里最常见的一种。

import { Session } from '../lib/harness.mjs'

const s = new Session('v0.10.0 界面筛选与分页')
await s.start()

const uniq = 'v010' + Math.random().toString(36).slice(2, 8)
const wait = (ms) => new Promise((r) => setTimeout(r, ms))

s.log('\n[1] 真实登录 admin')
await s.login()
s.check('登录 admin', true)

// ── 系统日志：新增的状态码与时间范围筛选 ──────────────────

s.log('\n[2] 系统日志页')
await s.goto('/system/log', 1200)

const controls = await s.evalJs(
  'const inputs = [...document.querySelectorAll("input")].map(i => i.placeholder || "");' +
  ' return {' +
  ' action: inputs.some(p => p.includes("操作")),' +
  ' username: inputs.some(p => p.includes("用户名")),' +
  ' statusCode: document.body.innerText.includes("状态码"),' +
  ' timeRange: inputs.some(p => p.includes("开始时间")) && inputs.some(p => p.includes("结束时间")),' +
  ' };',
)
s.check('操作/用户名筛选框在', controls.action && controls.username, JSON.stringify(controls))
s.check('状态码下拉在', controls.statusCode)
s.check('时间范围选择器在', controls.timeRange, JSON.stringify(controls))
await s.shot('v010-log-filters')

// 造一条可辨认的日志，再按用户名筛——判据是**每一行**都该是那个人。
// 顺带把这个角色删掉：它就是那条例子的来源，删完即等于清理掉了。
s.log('\n[3] 造一条日志并按用户名筛')
const role = await s.api('POST', '/api/admin/roles', { name: uniq + '_role' })
const roleId = role.body?.data?.id
const delRole = await s.api('DELETE', '/api/admin/roles/' + roleId)
s.check('造日志用的临时角色已删除', delRole.status === 200, 'status=' + delRole.status)
await wait(800) // 审计是异步落库的

await s.setInput('用户名', 'admin')
await s.clickByText('查询')
await wait(1200)

const filtered = await s.evalJs(
  'const rows = [...document.querySelectorAll(".n-data-table-tbody .n-data-table-tr")]' +
  '  .map(tr => (tr.querySelector("td")?.innerText || "").trim());' +
  ' return { total: rows.length, bad: rows.filter(t => t !== "admin") };',
)
s.check(
  '按用户名筛后每一行都是 admin',
  filtered.total > 0 && filtered.bad.length === 0,
  JSON.stringify(filtered),
)

// 筛不到人时应返回空列表，而不是继续展示全量
await s.setInput('用户名', 'zzz_nobody_zzz')
await s.clickByText('查询')
await wait(1200)
const empty = await s.evalJs(
  'return { rows: document.querySelectorAll(".n-data-table-tbody .n-data-table-tr").length,' +
  ' emptyText: document.body.innerText.includes("暂无数据") };',
)
s.check(
  '筛不到时列表为空（而非退回全量）',
  empty.rows === 0 || empty.emptyText,
  JSON.stringify(empty),
)
await s.shot('v010-log-empty')

// ── 角色管理：分页 ──────────────────────────────────────

s.log('\n[4] 角色管理页')

// 前置数据由套件**自己**造，不能指望库里的角色够多。
//
// 角色列表默认每页 10 条，而干净库只有 admin / user 两个角色，
// 压根不存在第二页——翻页断言会因为"没有第 2 页"而失败。
// 这个套件过去能过，只是因为跑它之前刚跑过集成测试，库被污染出了足够多的角色。
// 那是**偶然**，不是前提：换个干净库就红，而红的原因与被测的界面毫无关系。
// 计数从库里读，按当前总数补足，重复跑也不会越堆越多。
const PAGE_SIZE = 10
const count0 = await s.api('GET', '/api/admin/roles?page_size=1')
const have = count0.body?.data?.total ?? 0
const need = Math.max(0, PAGE_SIZE + 1 - have) // 至少让第二页存在
const fillerIds = []
for (let i = 0; i < need; i++) {
  const r = await s.api('POST', '/api/admin/roles', {
    name: `${uniq}_fill${String(i).padStart(2, '0')}`,
    description: 'v0.10.0 分页断言的前置数据',
  })
  if (r.status !== 200) {
    s.check('造分页前置角色', false, `status=${r.status} body=${JSON.stringify(r.body)}`)
    break
  }
  fillerIds.push(r.body?.data?.id)
}
s.check('分页前置角色已就绪', fillerIds.length === need,
  `原有 ${have} 个，补造 ${fillerIds.length}/${need} 个`)

await s.goto('/system/role', 1200)
const pager = await s.evalJs(
  'const p = document.querySelector(".n-pagination");' +
  ' return { hasPager: !!p,' +
  ' pagerText: p ? p.innerText.replace(/\\s+/g, " ").trim() : "",' +
  ' rows: document.querySelectorAll(".n-data-table-tbody .n-data-table-tr").length };',
)
s.check('分页控件在', pager.hasPager, JSON.stringify(pager))
s.check('默认每页 10 条，行数不超过', pager.rows > 0 && pager.rows <= 10, 'rows=' + pager.rows)
s.check('第一页填满（总数确实超过一页）', pager.rows === PAGE_SIZE, 'rows=' + pager.rows)
await s.shot('v010-role-pager')

// 翻到第二页，行内容应当变化（而不是被钉在第一页）
const firstCell = await s.evalJs(
  'return (document.querySelector(".n-data-table-tbody .n-data-table-tr td")?.innerText || "").trim();',
)
const clicked = await s.evalJs(
  'const items = [...document.querySelectorAll(".n-pagination .n-pagination-item")];' +
  ' const second = items.find(x => x.innerText.trim() === "2");' +
  ' if (!second) return false; second.click(); return true;',
)
await wait(1200)
const secondCell = await s.evalJs(
  'return (document.querySelector(".n-data-table-tbody .n-data-table-tr td")?.innerText || "").trim();',
)
s.check('第二页可点', clicked, 'pagerText=' + pager.pagerText)
s.check('翻到第二页后内容变化', firstCell !== secondCell, `${firstCell} -> ${secondCell}`)
await s.shot('v010-role-page2')

s.log('\n[5] 无残留')
// 分页前置角色按 created_at ASC 排在末尾，只读第一页查不到它们——
// 那种"查不到"会被误读成已清理。所以用 page_size=200 全量核对。
// 清理必须真的发生，不能只靠上面那条"查不到就算干净"——
// 那样一个"拒绝删除"的实现也能全绿。
let removed = 0
let removeFailed = []
for (const id of fillerIds) {
  const r = await s.api('DELETE', '/api/admin/roles/' + id)
  if (r.status === 200) removed++
  else removeFailed.push(id + '=' + r.status)
}
s.check('分页前置角色已逐个删除', removed === fillerIds.length,
  `删除 ${removed}/${fillerIds.length}` + (removeFailed.length ? ' 失败: ' + removeFailed.join(',') : ''))

// 核对放在删除**之后**：先查再删的话，删除动作本身从没被检验过，
// 一个"建了不删"的实现同样能全绿。
const leftover = await s.evalJs(
  'const r = await fetch("/api/admin/roles?page_size=200",' +
  ' { headers: { Authorization: "Bearer " + JSON.parse(decodeURIComponent(atob(localStorage.getItem("axum_token")))).value } });' +
  ' const j = await r.json();' +
  ' return (j.data?.items || []).filter(x => x.name === ' + JSON.stringify(uniq + '_role') +
  ' || x.name.startsWith(' + JSON.stringify(uniq + '_fill') + ')).length;',
)
s.check('临时角色没有残留', leftover === 0, '残留 ' + leftover + ' 个')

s.checkNoConsoleErrors([])
const failed = s.summary()
await s.stop()
process.exit(failed > 0 ? 1 : 0)
