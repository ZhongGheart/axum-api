/**
 * 菜单树辅助逻辑（角色授权弹窗用）
 *
 * 单独成文件而非留在 `.vue` 里：项目没有 `@vue/test-utils`，
 * 组件内部状态无法单测，把值得测的纯函数移出来才能被 vitest 覆盖。
 */

import type { MenuNode } from '@/api/menu'
import type { TreeOption } from 'naive-ui'

/** 菜单类型 → 中文标签 */
const TYPE_LABELS: Record<string, string> = {
  directory: '目录',
  menu: '菜单',
  button: '按钮',
}

/**
 * 授权树：把菜单节点转成 naive-ui `n-tree` 选项
 *
 * 保留 `type` 供 `renderLabel` 渲染类型徽标与权限码；
 * **不**按类型过滤——按钮型节点就是权限码，必须能在授权树里勾选。
 */
export function buildGrantTreeOptions(nodes: MenuNode[]): TreeOption[] {
  return nodes.map((node) => {
    const children = buildGrantTreeOptions(node.children ?? [])
    return {
      key: node.id,
      label: node.name,
      menuType: node.type,
      permission: node.permission,
      children: children.length ? children : undefined,
    }
  })
}

/** 展平菜单树，返回全部节点 ID（含 button） */
export function flattenMenuIds(nodes: MenuNode[]): string[] {
  const ids: string[] = []
  const walk = (list: MenuNode[]) => {
    for (const node of list) {
      ids.push(node.id)
      if (node.children?.length) walk(node.children)
    }
  }
  walk(nodes)
  return ids
}

/**
 * 从"某角色已授权菜单树"里取出已授权 ID 集合
 *
 * `GET /admin/menus?role_id=` 返回的是**过滤后**的树：父节点未授权时
 * 子节点会上浮成根节点（见后端 `build_tree(filtered, None)`）。
 * 这里只取 ID、**不用它的结构**渲染授权树，否则父子关系失真会让
 * `cascade` 连带勾选本不该勾的节点，造成静默扩权。
 */
export function authorizedMenuIds(granted: MenuNode[]): string[] {
  return [...new Set(flattenMenuIds(granted))]
}

/** 类型标签，未知类型回退为原值 */
export function menuTypeLabel(type: string | null | undefined): string {
  if (!type) return '未知'
  return TYPE_LABELS[type] ?? type
}

/**
 * 提交授权前的防御：剔除树里不存在的 ID
 *
 * 后端 `assign_role_menus` 已把非法 ID 变成整体 400，但前端先滤一遍，
 * 避免把"另一个页面的陈旧菜单列表"整体提交失败。
 */
export function filterKnownMenuIds(checked: Array<string | number>, tree: TreeOption[]): string[] {
  const known = new Set<string>()
  const walk = (options: TreeOption[]) => {
    for (const option of options) {
      if (typeof option.key === 'string') known.add(option.key)
      if (option.children?.length) walk(option.children as TreeOption[])
    }
  }
  walk(tree)
  return checked
    .map((key) => String(key))
    .filter((key) => known.has(key))
}
