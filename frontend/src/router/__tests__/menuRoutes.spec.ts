import { describe, expect, it, vi } from 'vitest'
import type { MenuNode } from '@/api/menu'
import { buildRoutesFromMenus, flattenMenus, resolveViewComponent } from '@/router/menuRoutes'

/** 构造菜单节点，字段可覆盖 */
function node(partial: Partial<MenuNode> & { name: string }): MenuNode {
  return {
    id: partial.id ?? partial.name,
    parent_id: partial.parent_id ?? null,
    name: partial.name,
    path: partial.path ?? null,
    component: partial.component ?? null,
    icon: partial.icon ?? null,
    sort_order: partial.sort_order ?? 0,
    type: partial.type ?? 'menu',
    permission: partial.permission ?? null,
    is_visible: partial.is_visible ?? true,
    children: partial.children ?? [],
  }
}

describe('resolveViewComponent', () => {
  it('解析真实存在的页面文件', () => {
    expect(resolveViewComponent('system/user/index')).toBeTypeOf('function')
    // 允许带 .vue 后缀与前置斜杠
    expect(resolveViewComponent('/system/user/index.vue')).toBeTypeOf('function')
  })

  it('组件不存在时返回 null（由调用方跳过并告警）', () => {
    expect(resolveViewComponent('not/exist/page')).toBeNull()
    expect(resolveViewComponent('')).toBeNull()
  })
})

describe('flattenMenus', () => {
  it('按深度优先展平树', () => {
    const tree = [
      node({
        name: '系统管理',
        path: '/system',
        children: [node({ name: '用户', path: '/system/user', component: 'system/user/index' })],
      }),
    ]
    expect(flattenMenus(tree).map((n) => n.name)).toEqual(['系统管理', '用户'])
  })
})

describe('buildRoutesFromMenus', () => {
  it('为带 component 的菜单生成路由并带标题', () => {
    const routes = buildRoutesFromMenus([
      node({ name: '用户管理', path: '/system/user', component: 'system/user/index' }),
    ])

    expect(routes).toHaveLength(1)
    expect(routes[0].path).toBe('/system/user')
    expect(routes[0].meta?.title).toBe('用户管理')
    expect(routes[0].component).toBeTypeOf('function')
  })

  it('目录节点重定向到首个可访问子页面（保持 /system 可直接访问）', () => {
    const routes = buildRoutesFromMenus([
      node({
        name: '系统管理',
        path: '/system',
        type: 'directory',
        children: [node({ name: '用户', path: '/system/user', component: 'system/user/index' })],
      }),
    ])

    const redirect = routes.find((r) => r.path === '/system')
    expect(redirect?.redirect).toBe('/system/user')
  })

  it('目录与其子页面同路径时只保留页面路由，不产生自跳转', () => {
    const routes = buildRoutesFromMenus([
      node({
        name: '组件示例',
        path: '/demo',
        type: 'directory',
        children: [node({ name: '前端组件', path: '/demo', component: 'demo/index' })],
      }),
    ])

    expect(routes.filter((r) => r.path === '/demo')).toHaveLength(1)
    expect(routes[0].redirect).toBeUndefined()
    expect(routes[0].component).toBeTypeOf('function')
  })

  it('组件文件缺失时跳过该菜单并告警，不影响其他路由', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const routes = buildRoutesFromMenus([
      node({ name: '坏菜单', path: '/broken', component: 'does/not/exist' }),
      node({ name: '首页', path: '/', component: 'home/index' }),
    ])

    expect(routes.map((r) => r.path)).toEqual(['/'])
    expect(warn).toHaveBeenCalledOnce()
    warn.mockRestore()
  })

  it('无 path 的菜单（如按钮型权限标记）不生成路由', () => {
    const routes = buildRoutesFromMenus([
      node({ name: '导出按钮', type: 'button', path: null, permission: 'user:export' }),
    ])
    expect(routes).toEqual([])
  })

  it('后端菜单种子里的每个 component 都能解析到真实页面文件', () => {
    // 该列表与 src/service/rbac.rs 的 SEED_MENUS_SQL 保持一致。
    // 重命名/删除页面文件却没有同步菜单种子时，这里会先失败，
    // 而不是等到上线后用户点菜单才发现页面打不开。
    const SEEDED_COMPONENTS = [
      'home/index',
      'demo/index',
      'demo/backend',
      'demo/dict',
      'system/user/index',
      'system/role/index',
      'system/menu/index',
      'system/log/index',
      'system/api-docs/index',
      'system/dict/index',
      'monitor/system/index',
      'monitor/api/index',
    ]

    const unresolved = SEEDED_COMPONENTS.filter((c) => resolveViewComponent(c) === null)
    expect(unresolved).toEqual([])
  })
})

describe('动态路由注册与撤销', () => {
  it('注册后该路径解析到菜单页，撤销后回落到 404', async () => {
    const { default: router, registerMenuRoutes, resetDynamicRoutes } = await import('@/router')

    const menu = { ...node({ name: '用户管理', path: '/system/user', component: 'system/user/index' }), id: 'u1' }
    registerMenuRoutes([menu])

    expect(router.hasRoute('menu-u1')).toBe(true)
    expect(router.resolve('/system/user').name).toBe('menu-u1')

    // 登出/换账号：撤销后不应再能访问该页面
    resetDynamicRoutes()
    expect(router.hasRoute('menu-u1')).toBe(false)

    // 撤销后该地址不再命中菜单路由，而是落到 404 通配路由
    const matched = router.resolve('/system/user').matched
    expect(matched.map((r) => r.name)).not.toContain('menu-u1')
    expect(matched.some((r) => r.path === '/:pathMatch(.*)*')).toBe(true)
  })
})
