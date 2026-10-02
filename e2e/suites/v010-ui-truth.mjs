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
await s.goto('/system/role', 1200)
const pager = await s.evalJs(
  'const p = document.querySelector(".n-pagination");' +
  ' return { hasPager: !!p,' +
  ' pagerText: p ? p.innerText.replace(/\\s+/g, " ").trim() : "",' +
  ' rows: document.querySelectorAll(".n-data-table-tbody .n-data-table-tr").length };',
)
s.check('分页控件在', pager.hasPager, JSON.stringify(pager))
s.check('默认每页 10 条，行数不超过', pager.rows > 0 && pager.rows <= 10, 'rows=' + pager.rows)
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
// 角色列表自 v0.10.0 起分页，临时角色按 created_at ASC 排在末尾，
// 只读第一页查不到它——那种"查不到"会被误读成已清理。
const leftover = await s.evalJs(
  'const r = await fetch("/api/admin/roles?page_size=200",' +
  ' { headers: { Authorization: "Bearer " + JSON.parse(decodeURIComponent(atob(localStorage.getItem("axum_token")))).value } });' +
  ' const j = await r.json();' +
  ' return (j.data?.items || []).filter(x => x.name === ' + JSON.stringify(uniq + '_role') + ').length;',
)
s.check('临时角色没有残留', leftover === 0, '残留 ' + leftover + ' 个')

s.checkNoConsoleErrors([])
const failed = s.summary()
await s.stop()
process.exit(failed > 0 ? 1 : 0)
