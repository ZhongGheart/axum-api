// v0.16.0 界面自查：字典的三个开关都是摆设
//
// 字典模块给了管理员三个控制：启用/禁用、设为默认、刷新缓存。
// 三个都在界面上正常工作，写入也都返回 200，但对实际行为没有任何影响。
// 所以这一版的判据不能是"接口 200"，必须是**业务页面真的变了**：
// 禁用项是否从下拉框里消失，禁用类型是否读不到，刷新提示里的数字是否属实。
//
// 用 demo 页上真实存在的 `custom_type` 下拉框验"禁用项从下拉消失"——
// 那正是 DictSelect 组件渲染的东西，也就是任意业务页面会看到的东西。

import { Session, sleep, waitFor } from '../lib/harness.mjs'

const s = new Session('v0.16.0 字典开关真的生效')

await s.start()

const CODE = 'custom_type'   // demo 页「自定义占位符」那一栏用的编码
const wait = (ms) => sleep(ms)

let typeId = null
const itemIds = {}

/** 造一份字典：三个项，丙是禁用的 */
async function seed() {
  const t = await s.api('POST', '/api/admin/dict/types', {
    code: CODE,
    name: 'v0.16 临时夹具',
    description: 'e2e 用例夹具，套件结束会删掉',
    status: 'enabled',
  })
  if (t.status !== 200) throw new Error('建字典类型失败: ' + JSON.stringify(t.body))
  typeId = t.body.data.id

  for (const [value, status, is_default] of [
    ['甲', 'enabled', true],
    ['乙', 'enabled', false],
    ['丙', 'disabled', false],
  ]) {
    const r = await s.api('POST', '/api/admin/dict/items', {
      dict_type_id: typeId,
      label: value,
      value,
      status,
      is_default,
    })
    if (r.status !== 200) throw new Error(`建字典项 ${value} 失败: ` + JSON.stringify(r.body))
    itemIds[value] = r.body.data.id
  }
}

/** 读读取端点返回的 value 列表 */
async function readValues(code = CODE) {
  const r = await s.api('GET', `/api/dict/${code}/items`)
  return { status: r.status, values: (r.body?.data || []).map((x) => x.value) }
}

/**
 * 打开 demo 页上的一个下拉框，读出真实渲染出来的选项文字
 *
 * 直接读接口只能证明后端返回对了，证明不了**界面**把禁用项藏起来了。
 * 而"业务页面的下拉框里还有那一项"才是这一版原本的缺陷。
 */
async function dropdownLabels(placeholder) {
  const opened = await s.evalJs(
    'const el = [...document.querySelectorAll(".n-base-selection")]'
    + `.find(x => (x.innerText || "").includes(${JSON.stringify(placeholder)}));`
    + ' if (!el) return false; el.click(); return true'
  )
  if (!opened) return null
  await wait(500)
  const labels = await s.evalJs(
    'const o = [...document.querySelectorAll(".n-base-select-option")];'
    + ' return o.map(x => (x.innerText || "").trim())'
  )
  await s.evalJs('document.body.click(); return true')
  await wait(200)
  return labels
}

// ── 0. 管理页本身必须渲染得出来 ──────────────────────────────

s.log('\n[0] 字典管理页是不是真的显示得出东西')
await s.login()

// 这一条是本套件的地基。写这条时它红过一次：页面用了 naive-ui 的
// `#left`/`#right` 命名插槽，而 n-split 只认编号插槽 `#1`/`#2`，
// 于是两个 pane 全空——整个字典管理页一片空白，菜单却照常能点进去。
// 后端接口一切正常，所以任何只看接口的检查都发现不了它。
await s.goto('/system/dict', 1500)
const pageShape = await s.evalJs(
  'return {'
  + ' pane1: (document.querySelector(".n-split-pane-1")?.innerText || "").trim(),'
  + ' pane2: (document.querySelector(".n-split-pane-2")?.innerText || "").trim(),'
  + ' cards: document.querySelectorAll(".dict-page .n-card").length'
  + ' }'
)
s.check('字典管理页左栏渲染出来了', pageShape.pane1.includes('字典类型'),
  'pane1="' + pageShape.pane1 + '"')
s.check('字典管理页右栏渲染出来了', pageShape.pane2.length > 0,
  'pane2="' + pageShape.pane2 + '"')
s.check('页面里有卡片而不是一片空白', pageShape.cards > 0,
  'cards=' + pageShape.cards)
await s.shot('v016-page-renders')

// ── 1. 禁用项必须从业务页面的下拉框里消失 ────────────────────

s.log('\n[1] 禁用项还留在下拉框里吗')
await seed()

const read1 = await readValues()
s.check('读取端点不再返回禁用项丙', !read1.values.includes('丙'),
  '返回=' + JSON.stringify(read1.values))
s.check('读取端点仍返回启用项', read1.values.includes('甲') && read1.values.includes('乙'),
  '返回=' + JSON.stringify(read1.values))

await s.goto('/demo/dict', 1500)
const labels = await dropdownLabels('请选择自定义类型')
s.check('demo 页该下拉框已打开（否则下面两条是空过）', !!labels,
  labels === null ? '没找到下拉框' : '已打开')
if (labels) {
  s.check('下拉框里有启用项甲', labels.some((l) => l.includes('甲')), JSON.stringify(labels))
  s.check('下拉框里没有禁用项丙——这正是原先的缺陷',
    !labels.some((l) => l.includes('丙')), JSON.stringify(labels))
}
await s.shot('v016-dropdown-hides-disabled')

// ── 2. 禁用类型：整份字典读不到 ─────────────────────────────

s.log('\n[2] 禁用整份字典后读取端点还读得到吗')
const disType = await s.api('PUT', `/api/admin/dict/types/${typeId}`, {
  code: CODE,
  name: 'v0.16 临时夹具',
  status: 'disabled',
})
s.check('禁用字典类型返回 200', disType.status === 200, 'status=' + disType.status)

const read2 = await readValues()
s.check('读取端点对已禁用类型返回空', read2.status === 200 && read2.values.length === 0,
  '返回=' + JSON.stringify(read2.values))

// 反向判据：重新启用后必须立刻能读到。若禁用时被写进了缓存，
// 管理员要等一小时 TTL 才能看到自己刚做的修改。
const reEnable = await s.api('PUT', `/api/admin/dict/types/${typeId}`, {
  code: CODE,
  name: 'v0.16 临时夹具',
  status: 'enabled',
})
const read3 = await readValues()
s.check('重新启用后立刻能读到（禁用期间的空结果没被缓存住）',
  reEnable.status === 200 && read3.values.length > 0,
  '返回=' + JSON.stringify(read3.values))

// ── 3. 默认项唯一 ──────────────────────────────────────────

s.log('\n[3] 连着把两项设为默认')
const d1 = await s.api('PUT', `/api/admin/dict/items/${itemIds['甲']}`, {
  label: '甲', value: '甲', status: 'enabled', is_default: true,
})
const d2 = await s.api('PUT', `/api/admin/dict/items/${itemIds['乙']}`, {
  label: '乙', value: '乙', status: 'enabled', is_default: true,
})
s.check('两次设默认都返回 200', d1.status === 200 && d2.status === 200,
  `${d1.status}/${d2.status}`)

const listed = await s.api('GET', `/api/admin/dict/items?dict_type_id=${typeId}`)
const defaults = (listed.body?.data || []).filter((x) => x.is_default).map((x) => x.value)
s.check('同一份字典只剩一个默认项', defaults.length === 1, '默认项=' + JSON.stringify(defaults))
s.check('默认项是后设的那个乙', defaults[0] === '乙', '默认项=' + JSON.stringify(defaults))

// 禁用项不能设默认：否则会出现"默认项指向一个业务页面看不到的值"
const d3 = await s.api('PUT', `/api/admin/dict/items/${itemIds['丙']}`, {
  label: '丙', value: '丙', status: 'disabled', is_default: true,
})
s.check('把禁用项设为默认被拒（400，且说清了原因）',
  d3.status === 400 && /禁用/.test(d3.body?.message || ''),
  'status=' + d3.status + ' msg=' + d3.body?.message)

// ── 4. 「刷新缓存」提示里的数字必须属实 ──────────────────────

s.log('\n[4] 点「刷新缓存」，提示必须报真实数字')
// 先让读取端把缓存打热，否则清的是 0 个键，验不出"是否真清"
await readValues()
await s.goto('/system/dict', 1500)

const apiRefresh = await s.api('POST', '/api/admin/dict/refresh')
s.check('刷新接口返回真实统计', apiRefresh.status === 200 &&
  typeof apiRefresh.body?.data?.cleared_keys === 'number',
  'status=' + apiRefresh.status + ' data=' + JSON.stringify(apiRefresh.body?.data))

await s.clickByText('刷新缓存')
await wait(900)
const toast = await waitFor(() => s.evalJs(
  'const m = [...document.querySelectorAll(".n-message")];'
  + ' return m.length ? m.map(x => x.innerText.replace(/\\s+/g, " ").trim()).join(" | ") : false'
), { timeout: 8000, label: '刷新缓存提示' }).catch(() => null)
s.check('界面上弹出了提示', !!toast, toast || '没弹提示')

if (toast) {
  s.check('提示不是无条件的"缓存刷新成功"',
    toast !== '缓存刷新成功', '实际="' + toast + '"')
  s.check('提示里报出了缓存键的真实数量',
    /没有需要清理的缓存键|已清空 \d+ 个缓存键/.test(toast), '实际="' + toast + '"')
  s.check('提示里报出了回填的类型数', /回填 \d+ 个类型/.test(toast),
    '实际="' + toast + '"')
}
await s.shot('v016-refresh-message')

// 反向判据：再点一次，确认接口仍然报得出数字，而不是写死的
const second = await s.api('POST', '/api/admin/dict/refresh')
s.check('第二次刷新仍报得出真实键数',
  second.status === 200 && typeof second.body?.data?.cleared_keys === 'number',
  JSON.stringify(second.body?.data))

// ── 5. 管理页要能看出哪些类型已被禁用 ────────────────────────

s.log('\n[5] 管理页对已禁用类型必须说清楚')
await s.api('PUT', `/api/admin/dict/types/${typeId}`, {
  code: CODE, name: 'v0.16 临时夹具', status: 'disabled',
})
await s.goto('/system/dict', 1500)
const typeRow = await s.evalJs(
  'const el = [...document.querySelectorAll(".dict-page .n-list-item")]'
  + '.find(x => (x.innerText || "").includes(' + JSON.stringify(CODE) + '));'
  + ' return el ? el.innerText.replace(/\\s+/g, " ").trim().slice(0, 200) : false'
).catch(() => false)
s.check('列表里能看出该类型已被禁用', !!typeRow && /已禁用/.test(typeRow),
  typeRow || '没找到该类型')
await s.shot('v016-disabled-type-row')

// ── 清理 ────────────────────────────────────────────────────

for (const v of ['甲', '乙', '丙']) {
  if (itemIds[v]) await s.api('DELETE', `/api/admin/dict/items/${itemIds[v]}`)
}
if (typeId) await s.api('DELETE', `/api/admin/dict/types/${typeId}`)
await s.api('POST', '/api/admin/dict/refresh')

const left = await s.api('GET', '/api/admin/dict/types')
const leftover = (left.body?.data || []).filter((x) => x.code === CODE)
s.check('套件没留下夹具', leftover.length === 0, '残留=' + JSON.stringify(leftover))

// 上面那条"把禁用项设为默认"是**故意**打到 400 的，浏览器必然记一条
// "Failed to load resource ... 400"。那是被断言过的预期失败，不是缺陷。
s.checkNoConsoleErrors(['400'])
const failed = s.summary()
await s.stop()
process.exit(failed > 0 ? 1 : 0)
