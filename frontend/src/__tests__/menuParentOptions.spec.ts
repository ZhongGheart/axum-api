/**
 * 编辑菜单时的上级候选项
 */
import { describe, expect, it } from 'vitest'
import type { MenuNode } from '@/api/menu'
import { buildParentOptions, collectSubtreeIds } from '@/utils/menu'

function node(partial: Partial<MenuNode> & { id: string }): MenuNode {
  return {
    id: partial.id,
    parent_id: null,
    name: partial.id,
    path: null,
    component: null,
    icon: null,
    sort_order: 0,
    type: partial.type ?? 'menu',
    permission: null,
    is_visible: true,
    children: partial.children ?? [],
  } as MenuNode
}


/**
 * 编辑菜单时的"上级菜单"候选项（v0.17.0）
 *
 * 为什么值得单测：候选列表一旦把节点自己的子树也放进去，
 * 用户就能在界面上造出一个环——而后端会拒绝它，于是表现为
 * "选项可选、提交必报错"的死路。把不可选项挡在候选之外，
 * 是让错误在**动手之前**就不可达。
 */
describe('buildParentOptions', () => {
  it('排除自身与自身整棵子树', () => {
    const tree = [
      { ...node({ id: 'a' }), name: 'A', children: [
        { ...node({ id: 'a1' }), name: 'A1', children: [
          { ...node({ id: 'a11' }), name: 'A11' },
        ] },
        { ...node({ id: 'a2' }), name: 'A2' },
      ] },
      { ...node({ id: 'b' }), name: 'B' },
    ]

    const options = buildParentOptions(tree, 'a')
    const keys = JSON.stringify(options)

    expect(keys).not.toContain('"a"')
    expect(keys).not.toContain('"a1"')
    expect(keys).not.toContain('"a11"')
    expect(keys).not.toContain('"a2"')
    // 兄弟节点必须还在——否则就无处可放了
    expect(keys).toContain('"b"')
  })

  it('只排除子树内部节点，祖先仍可选（把目录挂到更外层是合法操作）', () => {
    const tree = [
      { ...node({ id: 'root' }), name: 'ROOT', children: [
        { ...node({ id: 'mid' }), name: 'MID', children: [
          { ...node({ id: 'leaf' }), name: 'LEAF' },
        ] },
      ] },
      { ...node({ id: 'other' }), name: 'OTHER' },
    ]

    const options = buildParentOptions(tree, 'mid')
    const keys = JSON.stringify(options)

    expect(keys).not.toContain('"mid"')
    expect(keys).not.toContain('"leaf"')
    expect(keys).toContain('"root"')
    expect(keys).toContain('"other"')
  })

  it('新建时不排除任何节点', () => {
    const tree = [
      { ...node({ id: 'a' }), name: 'A', children: [{ ...node({ id: 'a1' }), name: 'A1' }] },
    ]
    const keys = JSON.stringify(buildParentOptions(tree))
    expect(keys).toContain('"a"')
    expect(keys).toContain('"a1"')
  })

  it('按钮型菜单不作为上级候选（它不参与导航层级）', () => {
    const tree = [
      { ...node({ id: 'btn' }), name: 'BTN', type: 'button' },
      { ...node({ id: 'dir' }), name: 'DIR' },
    ]
    const keys = JSON.stringify(buildParentOptions(tree))
    expect(keys).not.toContain('"btn"')
    expect(keys).toContain('"dir"')
  })

  it('排除集合会连带剔除按钮子树里的按钮', () => {
    const tree = [
      { ...node({ id: 'page' }), name: 'PAGE', children: [
        { ...node({ id: 'pbtn' }), name: 'PBTN', type: 'button' },
      ] },
    ]
    const keys = JSON.stringify(buildParentOptions(tree, 'page'))
    expect(keys).not.toContain('"page"')
    expect(keys).not.toContain('"pbtn"')
  })
})

describe('collectSubtreeIds', () => {
  it('收集目标节点及其所有后代', () => {
    const tree = [
      { ...node({ id: 'a' }), children: [
        { ...node({ id: 'b' }), children: [{ ...node({ id: 'c' }) }] },
      ] },
      { ...node({ id: 'd' }) },
    ]
    const ids = collectSubtreeIds(tree, 'b')
    expect([...ids].sort()).toEqual(['b', 'c'])
  })

  it('目标不存在时返回空集合（不该误伤整棵树）', () => {
    const tree = [{ ...node({ id: 'a' }) }, { ...node({ id: 'b' }) }]
    expect(collectSubtreeIds(tree, 'missing').size).toBe(0)
  })
})
