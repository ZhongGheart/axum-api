// 套件 1：权限码入口 + 监控页导出（v0.7.0 引入的回归）
//
// 这套的价值不在"按钮能不能点"，而在**前后端权限码是否一致**：
// 后端有码、前端没入口会让该能力永远用不上；前端有入口、后端没码
// 则表现为"按钮可见但一提交就 403"。

import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { Session, waitFor, sleep, ARTIFACTS } from '../lib/harness.mjs'

const s = new Session('权限码与监控导出')
const DOWNLOADS = join(ARTIFACTS, 'downloads')

await s.start()

// ── 1. 登录 ─────────────────────────────────────────────────
s.log('\n[1] 真实登录 admin')
await s.login()
s.check('登录 admin 并跳转首页', true, 'pathname=' + (await s.evalJs('return location.pathname')))

// storage.ts 编码：base64(encodeURIComponent(JSON.stringify({ value })))
const READ_TOKEN =
  '(() => { const raw = localStorage.getItem("axum_token");'
  + ' return raw ? JSON.parse(decodeURIComponent(atob(raw))).value : "" })()'
const tokenLen = await s.evalJs('const t = ' + READ_TOKEN + '; return t ? t.length : 0')
s.check('Token 已写入 localStorage', tokenLen > 100, tokenLen + ' chars')

// ── 2. 系统监控页 ────────────────────────────────────────────
s.log('\n[2] 系统监控页：导出按钮')
await s.goto('/system/monitor/system')
await waitFor(() => s.evalJs('return document.body.innerText.includes("系统监控")'),
  { label: '监控页' })
await sleep(1500)

const exportBtn = await s.evalJs(
  'const b = [...document.querySelectorAll("button")].find(x => x.textContent.includes("导出 Excel"));'
  + ' return b ? { found: true, disabled: b.disabled } : { found: false }'
)
s.check('系统监控页出现「导出 Excel」按钮', !!exportBtn.found,
  exportBtn.found ? 'disabled=' + exportBtn.disabled : '不在 DOM 中（v-permission 移除了？）')
await s.shot('01-monitor-system')

const codes = await s.evalJs(
  'const r = await fetch("/api/auth/permissions", { headers: { Authorization: "Bearer " + ' + READ_TOKEN + ' } });'
  + ' const j = await r.json();'
  + ' return j.data.filter(c => c === "system:monitor:export" || c === "system:test:access")'
)
s.check('页面会话持有两个新权限码', Array.isArray(codes) && codes.length === 2, JSON.stringify(codes))

// ── 3. 点击导出 ─────────────────────────────────────────────
s.log('\n[3] 点击导出 Excel')
s.resetBadResponses()
await s.clickByText('导出 Excel')

const toastOk = await waitFor(
  () => s.evalJs('return document.body.innerText.includes("导出成功")'),
  { timeout: 20000, label: '「导出成功」提示' }
).catch(() => false)
s.check('出现「导出成功」提示', !!toastOk)
await s.shot('02-monitor-export-done')

const file = await waitFor(() => {
  const f = readdirSync(DOWNLOADS).filter((x) => x.endsWith('.xlsx') && !x.endsWith('.crdownload'))
  return f.length ? f[0] : false
}, { timeout: 20000, label: 'xlsx 落盘' }).catch(() => null)
s.check('导出文件已落盘', !!file, file || '未找到 .xlsx')

if (file) {
  const buf = readFileSync(join(DOWNLOADS, file))
  const magic = buf.subarray(0, 2).toString('latin1')
  const listing = spawnSync('unzip', ['-l', join(DOWNLOADS, file)]).stdout.toString()
  const hasSheet = listing.includes('xl/worksheets/sheet1.xml')
  // 只看状态码会被 403 错误页骗过去：必须验证它真的是 xlsx
  s.check('内容是合法 xlsx（而非 403 错误页）', magic === 'PK' && hasSheet,
    'magic=' + magic + ' bytes=' + buf.length + ' sheet1=' + hasSheet)
}
s.checkNoUnexpectedHttp('导出过程无 4xx/5xx')

// ── 4. 后端能力示例页 ───────────────────────────────────────
s.log('\n[4] 后端能力示例页：能力探测')
await s.goto('/demo/backend')
await waitFor(() => s.evalJs('return document.body.innerText.includes("能力探测")'),
  { label: 'demo/backend' })
await sleep(700)

const cardTitles = await s.evalJs(
  'return [...document.querySelectorAll(".n-card")]'
  + '.map(c => (c.querySelector(".n-card-header__main") || {}).innerText || "")'
  + '.map(x => x.trim()).filter(Boolean)'
)
s.check('后端能力页有第 4 张卡「4. 能力探测」', cardTitles.includes('4. 能力探测'),
  JSON.stringify(cardTitles))
await s.shot('03-demo-backend')

s.resetBadResponses()
await s.clickByText('探测访问能力')
const probeText = await waitFor(() => s.evalJs(
  'const el = [...document.querySelectorAll("span")].find(x => x.textContent.includes("管理员访问成功"));'
  + ' return el ? el.innerText.trim() : false'
), { timeout: 20000, label: '能力探测结果' }).catch(() => null)
s.check('能力探测返回「管理员访问成功！」', !!probeText, probeText || '未拿到结果文本')
s.checkNoUnexpectedHttp('能力探测无 4xx/5xx')
await s.shot('04-demo-backend-probed')

await s.evalJs('window.scrollTo(0, document.body.scrollHeight); return true')
await sleep(600)
await s.shot('05-demo-card4-scrolled')

// 回归点：`fetchTest` 曾只挂在分页的 @update:page 上，首屏恒为空。
// 这个缺陷是人工核对截图发现的，CI 的 84 个前端单测完全看不到它。
const rows = await s.evalJs('return document.querySelectorAll(".n-data-table tbody tr").length')
const usersApi = await s.evalJs(
  'const r = await fetch("/api/admin/users?page=1&page_size=10", { headers: { Authorization: "Bearer " + '
  + READ_TOKEN + ' } });'
  + ' const j = await r.json();'
  + ' return { total: j.data && j.data.total, items: j.data && j.data.items && j.data.items.length }'
)
s.check('第 2 张卡分页表格首屏即有数据', rows > 0 && rows === usersApi.items,
  '行数=' + rows + ' /admin/users total=' + usersApi.total + ' items=' + usersApi.items)

// 本套件不故意触发任何 4xx/5xx（两处 checkNoUnexpectedHttp 都用默认空清单）
s.checkNoConsoleErrors([])
const failed = s.summary()
await s.stop()
process.exit(failed ? 1 : 0)
