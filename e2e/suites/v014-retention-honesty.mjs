// v0.14.0 界面自查：审计会过期，但没人被告知
//
// 这一版验的是"界面上说的话对不对"。保留天数、最早一条日志的时刻、
// 最近一次清理——这些措辞在边界情况下都能读通顺却在骗人，
// 所以每条断言都盯一个具体的骗法，而不是盯"页面上有段文字"。

import { Session, sleep, waitFor } from '../lib/harness.mjs'

const s = new Session('v0.14.0 保留策略如实说明')

await s.start()

const wait = (ms) => sleep(ms)

s.log('\n[1] 真实登录 admin')
await s.login()
s.check('登录 admin', true)

// ── 接口必须报出部署的真实策略 ──────────────────────────────

s.log('\n[2] 保留策略接口')
const info = await s.api('GET', '/api/admin/audit-logs/retention')
const d = info.body?.data
s.check('接口 200 且带出保留天数', info.status === 200 && typeof d?.retention_days === 'number',
  'status=' + info.status + ' body=' + JSON.stringify(d))

// enabled 与 retention_days 必须自洽：不能一边说启用一边报 0 天
s.check('enabled 与 retention_days 互相自洽',
  d && d.enabled === (d.retention_days > 0),
  'enabled=' + d?.enabled + ' days=' + d?.retention_days)

// 表里有日志时，最老时刻必须是字符串——它就是"还能查到多早"的真实答案
s.check('接口给出现存最早一条日志的时刻', typeof d?.oldest_log_at === 'string',
  'oldest_log_at=' + JSON.stringify(d?.oldest_log_at))

// ── 界面顶部如实说明 ────────────────────────────────────────

s.log('\n[3] 日志页顶部说明')
await s.goto('/system/log', 1200)

const banner = await waitFor(() => s.evalJs(
  'const a = [...document.querySelectorAll(".n-alert")].find(x => x.innerText.includes("日志"));'
  + ' return a ? a.innerText.replace(/\\s+/g, " ").trim() : false'
), { timeout: 15000, label: '保留策略说明' }).catch(() => null)
s.check('页面上有保留策略说明', !!banner, banner || '未找到')

if (banner) {
  s.check('说明里给出了保留天数', /\d+\s*天/.test(banner), banner)
  s.check('说明里给出了现存最早一条日志的时刻', banner.includes('现存最早一条'), banner)
  // 界面说的天数必须与接口一致：文案硬编码是最容易发生的漂移
  s.check('界面天数与接口一致',
    banner.includes(String(d?.retention_days)),
    `界面="${banner}" 接口=${d?.retention_days}`)
}
await s.shot('v014-retention-banner')

// ── 范围早于现存最早一条时必须提示 ──────────────────────────

s.log('\n[4] 筛到已被清理的时间范围时提示')

// 反向判据：还没设范围时**不该**出现该提示。
// 只断言"出现了"的话，一个常驻的提示条也能让这条绿——
// 而常驻提示会天天在眼前喊"数据可能丢了"，直到没人再读它为止。
const warnBefore = await s.evalJs(
  'const texts = [...document.querySelectorAll(".n-alert")].map(a => a.innerText);'
  + ' return texts.some(t => t.includes("早于") || t.includes("已被保留策略清理"));'
)
s.check('未设范围时不出现该提示（提示不是常驻的）', warnBefore === false,
  warnBefore ? '没设范围却已出现提示' : '未出现')

// 用一个远早于现存最早一条的范围（库里日志都是刚产生的）
const pad = (n) => String(n).padStart(2, '0')
const fmtLocal = (d) =>
  `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ` +
  `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
const from = new Date(Date.now() - 30 * 86400_000)
const to = new Date()
s.check('所选范围确实早于现存最早一条',
  from.getTime() < new Date(d.oldest_log_at).getTime(),
  `范围起点=${fmtLocal(from)} 最早一条=${d.oldest_log_at}`)

// 判定按**范围起点**而不是"结果为空"：范围 [30 天前, 今天] 在只剩
// 几小时日志时仍会返回非空结果，但那 30 天的数据一样是缺的。
// 只等空结果才提示的话，这里会安静地放过去。
await s.setInput('开始时间', fmtLocal(from))
await s.setInput('结束时间', fmtLocal(to))
await wait(400)
// naive-ui 的范围选择器在回车后才提交值
await s.evalJs(
  'const el = [...document.querySelectorAll("input")]'
  + '.find(i => i.placeholder && i.placeholder.includes("结束时间"));'
  + ' if (!el) return false;'
  + ' el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));'
  + ' el.blur(); return true'
)
await wait(600)
await s.clickByText('查询')
await wait(1500)

const warned = await s.evalJs(
  'const texts = [...document.querySelectorAll(".n-alert")].map(a => a.innerText);'
  + ' return texts.some(t => t.includes("早于") || t.includes("已被保留策略清理"));'
)
s.check('页面上提示了"所选范围早于现存最早一条"', warned,
  warned ? '' : '未出现提示：筛一个已被清理的区间却毫无说明')
await s.shot('v014-range-warning')

// ── 无残留 ──────────────────────────────────────────────────

// 本套件只读，不造数据
const tmp = await s.evalJs(
  'const t = localStorage.getItem("axum_token"); return t ? "有会话" : "无会话"'
)
s.check('套件未留下任何临时数据（本套件只读）', tmp === '有会话', tmp)

s.checkNoConsoleErrors([])
const failed = s.summary()
await s.stop()
process.exit(failed > 0 ? 1 : 0)
