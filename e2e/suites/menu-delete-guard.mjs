// 套件 2：菜单删除的授权下界（v0.8.0）
//
// `menus.parent_id` 声明了 ON DELETE CASCADE，删父目录会连带删掉子树里
// 承载权限码的按钮——所以守卫必须覆盖整棵子树，只查目标节点会漏掉这条路。
//
// 这套同时验证**反向**：无码菜单、以及"没人依赖"的码，仍然可以正常删除。
// 只测拒绝侧的话，一个"把所有删除都禁掉"的实现也能全绿。

import { Session } from '../lib/harness.mjs'

const s = new Session('菜单删除授权下界')
await s.start()

const uniq = 'v08' + Math.random().toString(36).slice(2, 8)

s.log('\n[1] 真实登录 admin')
await s.login()
s.check('登录 admin', true)

s.log('\n[2] 正常管理流程未被守卫误伤：无码页面菜单可删')
const page = await s.api('POST', '/api/admin/menus',
  { name: uniq + '_page', type: 'menu', path: '/' + uniq })
s.check('admin 新建无码页面菜单', page.status === 200, 'status=' + page.status)
const delPage = await s.api('DELETE', '/api/admin/menus/' + page.body?.data?.id)
s.check('admin 删除无码页面菜单 → 放行', delPage.status === 200,
  'status=' + delPage.status + ' ' + (delPage.body?.message || ''))

s.log('\n[3] 被别的角色依赖的码不可删')
const role = await s.api('POST', '/api/admin/roles', { name: uniq + '_role' })
const roleId = role.body?.data?.id
s.check('admin 新建临时角色', role.status === 200, 'status=' + role.status)

const code = 'probe:' + uniq + ':priv'
const btn = await s.api('POST', '/api/admin/menus',
  { name: uniq + '_btn', type: 'button', permission: code })
const btnId = btn.body?.data?.id
s.check('admin 新建带码按钮', btn.status === 200, 'status=' + btn.status)

const grant = await s.api('PUT', '/api/admin/roles/' + roleId + '/menus', { menu_ids: [btnId] })
s.check('把该码授予临时角色', grant.status === 200, 'status=' + grant.status)

s.resetBadResponses()
const delBtn = await s.api('DELETE', '/api/admin/menus/' + btnId)
s.check('admin 删除被他人依赖的码 → 被拒', delBtn.status === 403, 'status=' + delBtn.status)
s.log('  界面文案: ' + (delBtn.body?.message || '(无)'))
// 报错要能让人看懂缺的是哪个码，否则管理员无从下手
s.check('报错文案点明缺失的权限码',
  String(delBtn.body?.message || '').includes(code),
  delBtn.body?.message || '(无)')

// 判据落在数据上：被拒之后码必须还在
const tree = await s.api('GET', '/api/admin/menus')
s.check('被拒后按钮仍在树里（码未被剥掉）',
  JSON.stringify(tree.body?.data || '').includes(code))

s.log('\n[4] 恢复路径真实可用：撤销授权后即可删除')
const revoke = await s.api('PUT', '/api/admin/roles/' + roleId + '/menus', { menu_ids: [] })
s.check('撤销对该码的授权', revoke.status === 200, 'status=' + revoke.status)
const delBtn2 = await s.api('DELETE', '/api/admin/menus/' + btnId)
s.check('无人依赖后删除 → 放行', delBtn2.status === 200, 'status=' + delBtn2.status)

await s.api('DELETE', '/api/admin/roles/' + roleId)

s.log('\n[5] 菜单管理页真实渲染')
await s.goto('/system/menu')
await s.evalJs('return new Promise(r => setTimeout(r, 1200))')
s.check('菜单管理页渲染正常',
  (await s.evalJs('return document.body.innerText.includes("菜单")')) === true)
await s.shot('v08-menu-guard')

// 上面的 403 是本套件**预期**的，其余 4xx/5xx 才是意外。
// 同一份清单同时用于网络与控制台两处检查，避免两处声明漂移。
const EXPECTED_HTTP = ['403 ']
s.checkNoConsoleErrors(EXPECTED_HTTP)
s.checkNoUnexpectedHttp('除预期的 403 外无 4xx/5xx', EXPECTED_HTTP)

const failed = s.summary()
await s.stop()
process.exit(failed ? 1 : 0)
