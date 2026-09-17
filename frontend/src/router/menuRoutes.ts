/**
 * 菜单 → 路由 转换
 *
 * 后端菜单是导航的唯一来源；本模块把菜单树转成 vue-router 路由记录。
 * 组件用 `import.meta.glob` 解析（Vite 要求导入路径可在构建期静态分析）。
 */
import type { RouteRecordRaw } from 'vue-router'
import type { MenuNode } from '@/api/menu'

/** src/views 下所有页面模块，key 形如 `../views/system/user/index.vue` */
const viewModules = import.meta.glob('../views/**/*.vue')

/**
 * 把菜单里的 `component` 标识解析为懒加载组件
 *
 * `component` 形如 `system/user/index`（相对 `src/views`，可带或不带 `.vue`）。
 * 解析不到返回 null，由调用方告警并跳过。
 */
export function resolveViewComponent(component: string): (() => Promise<unknown>) | null {
  const normalized = component.trim().replace(/^\/+/, '').replace(/\.vue$/, '')
  if (!normalized) return null
  const loader = viewModules[`../views/${normalized}.vue`]
  return (loader as (() => Promise<unknown>)) ?? null
}

/** 深度优先展平菜单树 */
export function flattenMenus(nodes: MenuNode[], out: MenuNode[] = []): MenuNode[] {
  for (const node of nodes) {
    out.push(node)
    if (node.children?.length) flattenMenus(node.children, out)
  }
  return out
}

/**
 * 由菜单树生成路由记录
 *
 * - 有 `component` 的节点生成页面路由（绝对路径，挂在 MainLayout 之下）
 * - 无 `component` 的目录节点，若该路径没有页面，则重定向到首个可访问子页面，
 *   使 `/system` 这类目录地址仍可直接访问
 * - 组件文件不存在时跳过并告警，避免一条坏菜单导致整张路由表注册失败
 */
export function buildRoutesFromMenus(menus: MenuNode[]): RouteRecordRaw[] {
  const nodes = flattenMenus(menus)
  const routes: RouteRecordRaw[] = []
  const registered = new Set<string>()

  for (const node of nodes) {
    if (!node.path || !node.component || registered.has(node.path)) continue

    const component = resolveViewComponent(node.component)
    if (!component) {
      console.warn(`[menu] 菜单「${node.name}」引用了不存在的组件: ${node.component}`)
      continue
    }

    registered.add(node.path)
    routes.push({
      path: node.path,
      name: `menu-${node.id}`,
      component,
      meta: { title: node.name, menuId: node.id },
    })
  }

  for (const node of nodes) {
    if (!node.path || node.component || registered.has(node.path)) continue

    const childPath = flattenMenus(node.children ?? []).find((child) => child.path && child.component)
      ?.path
    if (!childPath || childPath === node.path) continue

    registered.add(node.path)
    routes.push({ path: node.path, redirect: childPath })
  }

  return routes
}
