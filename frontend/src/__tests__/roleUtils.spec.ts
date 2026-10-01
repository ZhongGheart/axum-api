/**
 * 用户表单角色多选框的选项拼装
 *
 * 覆盖 v0.5.0 PR-2 拆掉前端硬编码 [admin, user] 之后的两个真实风险：
 * 1. **前端不得自行归一化角色名**——归一化的唯一数据源在后端，
 *    前端再 lower() 一次只会让两边规则漂移；
 * 2. **历史角色的当前值不能悄悄消失**——迁移 `008` 会刻意保留归一化后
 *    撞名的旧角色名，若下拉里没有它，管理员看不出自己正在编辑什么角色。
 *
 * v0.6.0 再加一条：**多角色用户的全部角色都必须回填**。
 * 此前只回填 `roles[0]` 并整体覆盖提交，用户其余角色被静默删除——
 * 这是本项要修的核心缺陷，测试就钉在这里。
 */
import { describe, expect, it } from 'vitest'
import {
  buildRoleSelectOptions,
  currentRoleNames,
  pickDefaultRoles,
  DEFAULT_ROLE_NAME,
  type RoleListItem,
} from '@/utils/role'

const builtin: RoleListItem[] = [
  { name: 'admin', description: '系统管理员，拥有所有权限' },
  { name: 'user', description: null },
]

describe('buildRoleSelectOptions', () => {
  it('下拉内容完全由接口返回的角色决定，不掺入任何内置默认值', () => {
    const options = buildRoleSelectOptions([{ name: 'auditor', description: null }])
    expect(options).toEqual([{ label: 'auditor', value: 'auditor' }])
  })

  it('有描述时把描述拼进 label，没有描述时只显示角色名', () => {
    const options = buildRoleSelectOptions(builtin)
    expect(options[0].label).toBe('admin（系统管理员，拥有所有权限）')
    expect(options[1].label).toBe('user')
  })

  it('value 原样透传角色名，不在前端做大小写/空白归一化', () => {
    const options = buildRoleSelectOptions([{ name: ' Auditor ', description: null }])
    expect(options[0].value).toBe(' Auditor ')
  })

  it('当前角色不在列表里时补一个显式选项，而不是让它消失', () => {
    const options = buildRoleSelectOptions(builtin, ['LegacyRole'])
    const injected = options.find((o) => o.value === 'LegacyRole')
    expect(injected).toBeDefined()
    expect(injected!.label).toContain('已不在角色列表中')
  })

  it('当前角色已在列表里时不重复注入', () => {
    const options = buildRoleSelectOptions(builtin, ['user'])
    expect(options).toHaveLength(2)
    expect(options.filter((o) => o.value === 'user')).toHaveLength(1)
  })

  it('多个当前角色都不在列表里时逐个补齐，不只补第一个', () => {
    const options = buildRoleSelectOptions(builtin, ['LegacyA', 'user', 'LegacyB'])
    const values = options.map((o) => o.value)
    expect(values).toContain('LegacyA')
    expect(values).toContain('LegacyB')
    expect(values).toContain('user')
  })

  it('未传当前角色时不注入任何兜底项', () => {
    expect(buildRoleSelectOptions(builtin)).toHaveLength(2)
  })
})

describe('pickDefaultRoles', () => {
  it('优先选中普通用户', () => {
    expect(pickDefaultRoles(builtin)).toEqual([DEFAULT_ROLE_NAME])
  })

  it('没有普通用户时退回列表首个，而不是空值', () => {
    expect(pickDefaultRoles([{ name: 'admin', description: null }])).toEqual(['admin'])
  })

  it('角色为空时返回空数组，交给表单必填校验报错', () => {
    expect(pickDefaultRoles([])).toEqual([])
  })
})

describe('currentRoleNames', () => {
  it('取真实角色集合，而不是两值展示枚举', () => {
    // 后端 UserInfo.role 对任何非 admin 角色都返回 "user"
    expect(currentRoleNames({ role: 'user', roles: ['auditor'] })).toEqual(['auditor'])
  })

  it('核心回归：多角色必须全部回填，不能只取第一个', () => {
    // 这正是"保存后其余角色被静默删除"的根因
    expect(currentRoleNames({ role: 'admin', roles: ['admin', 'auditor'] })).toEqual([
      'admin',
      'auditor',
    ])
  })

  it('角色集合为空时退回展示枚举（兼容老响应）', () => {
    expect(currentRoleNames({ role: 'admin' })).toEqual(['admin'])
    expect(currentRoleNames({ role: 'admin', roles: [] })).toEqual(['admin'])
  })

  it('两个字段都缺失时返回空数组', () => {
    expect(currentRoleNames({})).toEqual([])
  })

  it('返回副本而非原数组引用，调用方改动不会污染响应对象', () => {
    const user = { role: 'admin', roles: ['admin'] }
    const names = currentRoleNames(user)
    names.push('injected')
    expect(user.roles).toEqual(['admin'])
  })
})
