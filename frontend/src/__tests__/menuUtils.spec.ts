/**
 * 角色授权树辅助逻辑
 *
 * 覆盖两个真实风险点：
 * 1. `authorizedMenuIds` 必须只取 ID、不依赖过滤后树的层级结构；
 * 2. `filterKnownMenuIds` 必须剔除树里不存在的 ID，
 *    否则一次陈旧勾选就让后端整单 400（v0.5.0 起授权写路径不再吞错）。
 */
import { describe, expect, it } from 'vitest'
import type { MenuNode } from '@/api/menu'
import {
  buildGrantTreeOptions,
  flattenMenuIds,
  authorizedMenuIds,
  menuTypeLabel,
  filterKnownMenuIds,
} from '@/utils/menu'

function node(partial: Partial<MenuNode> & { id: string }): MenuNode {
  return {
    parent_id: null,
    name: partial.id,
    path: null,
    component: null,
    icon: null,
    sort_order: 0,
    type: 'menu',
    permission: null,
    is_visible: true,
    children: [],
    ...partial,
  }
}

/** 完整菜单树：目录 → 菜单 → 按钮（权限码） */
const fullTree: MenuNode[] = [
  node({
    id: 'sys',
    name: '系统管理',
    type: 'directory',
    children: [
      node({
        id: 'role',
        name: '角色管理',
        path: '/system/role',
        children: [
          node({ id: 'role-create', name: '新建角色', type: 'button', permission: 'system:role:create' }),
          node({ id: 'role-delete', name: '删除角色', type: 'button', permission: 'system:role:delete' }),
        ],
      }),
    ],
  }),
]

describe('buildGrantTreeOptions', () => {
  it('保留 button 节点（权限码必须能在授权树里勾选）', () => {
    const options = buildGrantTreeOptions(fullTree)
    const roleNode = options[0].children?.[0] as { children?: Array<{ key: string }> }
    expect(roleNode.children?.map((c) => c.key)).toEqual(['role-create', 'role-delete'])
  })

  it('把 type 与 permission 挂在节点上供 renderLabel 使用', () => {
    const options = buildGrantTreeOptions(fullTree)
    const roleNode = options[0].children?.[0] as {
      children?: Array<{ permission?: string | null; menuType?: string }>
    }
    expect(roleNode.children?.[0]).toMatchObject({
      menuType: 'button',
      permission: 'system:role:create',
    })
  })

  it('叶子节点不带空 children 数组', () => {
    const options = buildGrantTreeOptions([node({ id: 'solo' })])
    expect(options[0].children).toBeUndefined()
  })

  it('空树返回空数组', () => {
    expect(buildGrantTreeOptions([])).toEqual([])
  })
})

describe('flattenMenuIds / authorizedMenuIds', () => {
  it('深度优先展平出全部节点 ID', () => {
    expect(flattenMenuIds(fullTree)).toEqual(['sys', 'role', 'role-create', 'role-delete'])
  })

  it('children 缺失时不抛异常', () => {
    const orphan = node({ id: 'x' })
    delete (orphan as { children?: unknown }).children
    expect(flattenMenuIds([orphan])).toEqual(['x'])
  })

  it('过滤后的树里子节点上浮成根，也能全部取出', () => {
    // 后端 build_tree(filtered, None) 的典型输出：父未授权，子上浮
    const floated: MenuNode[] = [node({ id: 'role-delete', name: '删除角色', type: 'button' })]
    expect(authorizedMenuIds(floated)).toEqual(['role-delete'])
  })

  it('结果去重', () => {
    expect(authorizedMenuIds([node({ id: 'a' }), node({ id: 'a' })])).toEqual(['a'])
  })

  it('未授权任何菜单时为空数组（可提交空集合撤销全部授权）', () => {
    expect(authorizedMenuIds([])).toEqual([])
  })
})

describe('menuTypeLabel', () => {
  it('已知类型映射为中文', () => {
    expect(menuTypeLabel('directory')).toBe('目录')
    expect(menuTypeLabel('menu')).toBe('菜单')
    expect(menuTypeLabel('button')).toBe('按钮')
  })

  it('未知类型回退为原值，缺失时为「未知」', () => {
    expect(menuTypeLabel('weird')).toBe('weird')
    expect(menuTypeLabel(null)).toBe('未知')
    expect(menuTypeLabel(undefined)).toBe('未知')
  })
})

describe('filterKnownMenuIds', () => {
  const tree = buildGrantTreeOptions(fullTree)

  it('保留树中存在的 ID', () => {
    expect(filterKnownMenuIds(['sys', 'role'], tree)).toEqual(['sys', 'role'])
  })

  it('剔除树中不存在的陈旧 ID，避免整单 400', () => {
    expect(filterKnownMenuIds(['sys', 'ghost-menu'], tree)).toEqual(['sys'])
  })

  it('数字型 key（naive-ui 可能给 number）统一转字符串后按树判定', () => {
    expect(filterKnownMenuIds([1, 2, 3], tree)).toEqual([])
  })

  it('保持提交顺序，不重排', () => {
    expect(filterKnownMenuIds(['role', 'sys', 'role-create'], tree)).toEqual([
      'role',
      'sys',
      'role-create',
    ])
  })

  it('全部失效时返回空数组（等于撤销全部授权，是合法请求）', () => {
    expect(filterKnownMenuIds(['ghost'], tree)).toEqual([])
  })
})
