// 套件：菜单层级真的可调（v0.17.0）
//
// v0.16.0 之前有两处让"调整菜单层级"这件事在界面上根本做不到，
// 且其中一处会在管理员不知情时**静默改结构**：
//
// 1. 编辑弹窗里根本没有"上级菜单"字段，但提交时却会带上 `parentId` 的
//    **上次残留值**——先在 A 节点点"新增子菜单"、再点 B 节点"编辑"保存，
//    B 就被挂到 A 下了，界面上没有任何东西提示这件事。
// 2. 即使直接调 API 传 `parent_id: null`（想摘成根），后端也当成"本次不改"：
//    返回 200、字段原样回显，而结构纹丝不动。
//
// 这套用**真实浏览器**走一遍界面，断言落在"操作之后数据真的变了"上。

import { Session } from '../lib/harness.mjs'

const s = new Session('菜单层级可调')
await s.start()

const uniq = 'v17' + Math.random().toString(36).slice(2, 8)
const made = []
const wait = (ms) => new Promise((r) => setTimeout(r, ms))

/** 走接口建临时菜单并记下来，最后统一清理 */
async function mk(parent, name, type = 'directory') {
  const r = await s.api('POST', '/api/admin/menus', {
    parent_id: parent || undefined, name, type,
  })
  s.check('建临时菜单 ' + name, r.status === 200,
    'status=' + r.status + ' ' + (r.body?.message || ''))
  const id = r.body?.data?.id
  made.push(id)
  return id
}

/** 从菜单树里找某个 id 的节点 */
async function nodeById(id) {
  const t = await s.api('GET', '/api/admin/menus')
  const found = []
  const walk = (ns) => { for (const n of ns || []) { found.push(n); walk(n.children) } }
  walk(t.body?.data)
  return found.find((n) => n.id === id)
}

/**
 * 在菜单树上找到某节点那一行的第 n 个图标按钮（0=新增子 1=编辑 2=删除）
 *
 * `:not(.n-tree-node-content__text)` 不能省：naive-ui 的行容器与内层文字 span
 * 类名互为前缀，朴素选择器会同时命中两层，取到内层就找不到按钮了。
 * 行标签也读 `.n-tree-node-content__text`（并没有 `.n-tree-node-label` 这个类）。
 */
const rowButton = (name, nth) => `
  const rows = [...document.querySelectorAll('.n-tree-node-content:not(.n-tree-node-content__text)')];
  const row = rows.find(r => (r.querySelector('.n-tree-node-content__text')?.innerText || '').trim() === ${JSON.stringify(name)});
  if (!row) return 'no-row';
  const btns = row.querySelectorAll('button');
  if (btns.length <= ${nth}) return 'no-btn:' + btns.length;
  btns[${nth}].click();
  return 'clicked';
`

/** 打开某节点的编辑弹窗 */
async function openEdit(name) {
  const r = await s.evalJs(rowButton(name, 1))
  await wait(700)
  const opened = await s.evalJs(
    'return document.querySelector(".n-modal")?.innerText.includes("编辑菜单") === true')
  return { r, opened }
}

/** 关掉当前弹窗 */
async function closeModal() {
  await s.evalJs(
    'const btns = [...document.querySelectorAll(".n-modal .n-card__footer button")];'
    + ' btns[0]?.click(); return true')
  await wait(400)
}

/** 点"保存" */
async function submitModal() {
  await s.evalJs(
    'const btns = [...document.querySelectorAll(".n-modal .n-card__footer button")];'
    + ' btns[btns.length - 1].click(); return true')
  await wait(1200)
}

/** 展开"上级菜单"下拉，返回候选名列表 */
async function openParentOptions() {
  await s.evalJs(
    'const m = document.querySelector(".n-modal");'
    + ' const it = [...m.querySelectorAll(".n-form-item")]'
    + '   .find(i => (i.querySelector(".n-form-item-label")?.innerText || "").trim() === "上级菜单");'
    + ' it?.querySelector(".n-base-selection")?.click(); return true')
  await wait(800)
  // 选项在 `.n-tree-select-menu` 里；不加这个限定会命中弹窗背后的整棵页面树，
  // 断言就会变成"树里有这几个名字"这种恒真判断，等于没测。
  return s.evalJs(
    'return [...document.querySelectorAll(".n-tree-select-menu .n-tree-node-content__text")]'
    + '   .map(e => e.innerText.trim());')
}

await s.login()
s.check('登录 admin', true)

const dirA = await mk(null, uniq + '_A')
const dirB = await mk(null, uniq + '_B')
const childOfA = await mk(dirA, uniq + '_A1')
// B 会在第 [3] 步被改名。树上的**名字**跟着变，后面按名字找它的地方
// 必须用这个变量，否则会拿着旧名字去找一个已经不存在的东西——
// 那种失败看起来像"功能没生效"，其实是自己找错了对象。
let nameB = uniq + '_B'

// ──────────────────────────────────────────────
s.log('\n[1] 编辑弹窗里必须有"上级菜单"字段')

await s.goto('/system/menu', 1500)
s.check('菜单页渲染出来了',
  (await s.evalJs('return document.body.innerText.includes("菜单管理")')) === true)
await s.shot('v017-menu-page')

const { opened } = await openEdit(uniq + '_A')
s.check('点编辑能打开弹窗', opened === true)

const labels = await s.evalJs(
  'return [...document.querySelectorAll(".n-modal .n-form-item-label__text")]'
  + '   .map(e => e.innerText.trim());'
)
s.check('弹窗里有"上级菜单"字段', labels.includes('上级菜单'),
  '实际字段=' + JSON.stringify(labels))
await s.shot('v017-edit-has-parent-field')
await closeModal()

// ──────────────────────────────────────────────
s.log('\n[2] 上级候选里不能有自己，也不能有自己的下级')

await openEdit(uniq + '_A')
const opts = await openParentOptions()
s.log('  上级候选: ' + JSON.stringify(opts))
s.check('下拉真的展开了（判据本身有效）', opts.length > 0, JSON.stringify(opts))
s.check('候选里没有自己', !opts.includes(uniq + '_A'), JSON.stringify(opts))
s.check('候选里没有自己的子树', !opts.includes(uniq + '_A1'), JSON.stringify(opts))
s.check('兄弟目录仍在候选里', opts.includes(nameB), JSON.stringify(opts))
await s.shot('v017-parent-options')
await s.evalJs('document.body.click(); return true')
await wait(300)
await closeModal()

// ──────────────────────────────────────────────
s.log('\n[3] 脏状态泄漏：先"新增子菜单"再"编辑"，被编辑的菜单不该被挪走')

// 先在 A 上点"新增子菜单"，把 parentId 污染成 A
await s.evalJs(rowButton(uniq + '_A', 0))
await wait(700)
await closeModal()

// 现在去编辑 B —— 旧实现会把 B 静默挂到 A 下
await openEdit(uniq + '_B')
const shownParent = await s.evalJs(
  'const m = document.querySelector(".n-modal");'
  + ' const it = [...m.querySelectorAll(".n-form-item")]'
  + '   .find(i => (i.querySelector(".n-form-item-label")?.innerText || "").trim() === "上级菜单");'
  + ' const ph = it?.querySelector(".n-base-selection-placeholder");'
  + ' return ph ? ph.innerText.trim() : (it?.querySelector(".n-base-selection-label")?.innerText.trim() || "");'
)
s.check('编辑 B 时弹窗显示的上级是它本来的上级（根，未被污染成 A）',
  shownParent === '' || shownParent.includes('不选则为顶级菜单'),
  '实际="' + shownParent + '"')

// 只改名字保存，完全不碰上级
const newName = uniq + '_B_renamed'
const typed = await s.evalJs(
  'const items = [...document.querySelectorAll(".n-modal .n-form-item")];'
  + ' const item = items.find(i => (i.querySelector(".n-form-item-label__text")?.innerText || "").trim() === "菜单名称");'
  + ' const field = item?.querySelector("input");'
  + ' if (!field) return false;'
  // 局部变量**不能**叫 `name`：那会遮蔽全局 `window.name`，
  // 原生 setter 以它为 this 就抛 Illegal invocation
  + ' const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;'
  + ' setter.call(field, ' + JSON.stringify(newName) + ');'
  + ' field.dispatchEvent(new Event("input", { bubbles: true }));'
  + ' field.dispatchEvent(new Event("change", { bubbles: true }));'
  + ' return true'
)
s.check('改得了菜单名称', typed === true)
await submitModal()

const bAfter = await nodeById(dirB)
s.check('B 仍然挂在根下，没被静默挪到 A 下',
  bAfter?.parent_id === null,
  '实际 parent_id=' + JSON.stringify(bAfter?.parent_id))
s.check('改名确实生效了', bAfter?.name === newName, '实际=' + bAfter?.name)
nameB = newName
await s.shot('v017-after-rename')

// ──────────────────────────────────────────────
s.log('\n[4] 真的能通过界面把菜单挪到别的目录下')

await openEdit(uniq + '_A1')
await openParentOptions()
// 点**行容器**而不是里面的文字 span：naive-ui 的选中事件绑在
// `.n-tree-node-content` 上，只点 span 不会选中。
const picked = await s.evalJs(
  'const os = [...document.querySelectorAll(".n-tree-select-menu .n-tree-node-content")];'
  + ' const hit = os.find(o => (o.querySelector(".n-tree-node-content__text")?.innerText || "").trim()'
  + '   === ' + JSON.stringify(nameB) + ');'
  + ' if (!hit) return "no-option:" + os.length;'
  + ' hit.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));'
  + ' hit.click(); return true')
s.check('在下拉里选中了目标目录', picked === true,
  'picked=' + JSON.stringify(picked))
await wait(500)
await submitModal()

const moved = await nodeById(childOfA)
s.check('A1 真的挂到了 B 下', moved?.parent_id === dirB,
  '实际 parent_id=' + moved?.parent_id)
await s.shot('v017-moved')

// ──────────────────────────────────────────────
s.log('\n[5] 结构校验在接口层也拦得住（界面只是少给选项，闸门在后端）')

// 前置显式做成 A → A1 → B（A1 的上级是 B）。
// 不依赖第 [4] 步是否成功——否则界面一失败，这里的判据就悄悄失效了。
const setup = await s.api('PUT', '/api/admin/menus/' + childOfA, { parent_id: dirB })
s.check('前置：A1 的上级是 B', setup.status === 200,
  'status=' + setup.status + ' ' + (setup.body?.message || ''))

s.resetBadResponses()
s.forgetDeliberateFailures()

const selfRef = await s.api('PUT', '/api/admin/menus/' + dirB, { parent_id: dirB })
s.check('把目录设为自己的上级 → 400', selfRef.status === 400,
  'status=' + selfRef.status + ' ' + (selfRef.body?.message || ''))
s.check('报错说清了是自引用', String(selfRef.body?.message || '').includes('它自己'),
  selfRef.body?.message || '(无)')

const cycle = await s.api('PUT', '/api/admin/menus/' + dirB, { parent_id: childOfA })
s.check('挂到自己的子孙下 → 400', cycle.status === 400,
  'status=' + cycle.status + ' ' + (cycle.body?.message || ''))
s.check('报错说清了会成环', String(cycle.body?.message || '').includes('成环'),
  cycle.body?.message || '(无)')

const ghost = await s.api('PUT', '/api/admin/menus/' + dirB,
  { parent_id: '00000000-0000-4000-8000-0000000000ff' })
s.check('挂到不存在的上级 → 400 而不是 500', ghost.status === 400,
  'status=' + ghost.status)

// 被拒之后结构必须原封不动
const stillB = await nodeById(dirB)
s.check('连续三次被拒后 B 的上级没变', stillB?.parent_id === null,
  '实际=' + stillB?.parent_id)

// ──────────────────────────────────────────────
s.log('\n[6] 清场')
for (const id of made.reverse()) {
  await s.api('DELETE', '/api/admin/menus/' + id)
}
const left = await s.api('GET', '/api/admin/menus')
s.check('套件没留下临时菜单',
  !JSON.stringify(left.body?.data || '').includes(uniq))

// 第 [5] 步是故意打到 400 的，浏览器必然记 "Failed to load resource ... 400"
s.checkNoConsoleErrors(['400'])

const failed = s.summary()
await s.stop()
process.exit(failed ? 1 : 0)
