/**
 * 用户表单角色下拉的选项拼装
 *
 * 覆盖 v0.5.0 PR-2 拆掉前端硬编码 [admin, user] 之后的两个真实风险：
 * 1. **前端不得自行归一化角色名**——归一化的唯一数据源在后端，
 *    前端再 lower() 一次只会让两边规则漂移；
 * 2. **历史角色的当前值不能悄悄消失**——迁移 `008` 会刻意保留归一化后
 *    撞名的旧角色名，若下拉里没有它，管理员看不出自己正在编辑什么角色。
 */
import { describe, expect, it } from 'vitest'
import {
  buildRoleSelectOptions,
  currentRoleName,
  pickDefaultRole,
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
    const options = buildRoleSelectOptions(builtin, 'LegacyRole')
    const injected = options.find((o) => o.value === 'LegacyRole')
    expect(injected).toBeDefined()
    expect(injected!.label).toContain('已不在角色列表中')
  })

  it('当前角色已在列表里时不重复注入', () => {
    const options = buildRoleSelectOptions(builtin, 'user')
    expect(options).toHaveLength(2)
    expect(options.filter((o) => o.value === 'user')).toHaveLength(1)
  })

  it('未传当前角色时不注入任何兜底项', () => {
    expect(buildRoleSelectOptions(builtin)).toHaveLength(2)
  })
})

describe('pickDefaultRole', () => {
  it('优先选中普通用户', () => {
    expect(pickDefaultRole(builtin)).toBe(DEFAULT_ROLE_NAME)
  })

  it('没有普通用户时退回列表首个，而不是空值', () => {
    expect(pickDefaultRole([{ name: 'admin', description: null }])).toBe('admin')
  })

  it('角色为空时返回空串，交给表单必填校验报错', () => {
    expect(pickDefaultRole([])).toBe('')
  })
})

describe('currentRoleName', () => {
  it('取真实角色集合，而不是两值展示枚举', () => {
    // 后端 UserInfo.role 对任何非 admin 角色都返回 "user"
    expect(currentRoleName({ role: 'user', roles: ['auditor'] })).toBe('auditor')
  })

  it('角色集合为空时退回展示枚举', () => {
    expect(currentRoleName({ role: 'admin' })).toBe('admin')
    expect(currentRoleName({ role: 'admin', roles: [] })).toBe('admin')
  })

  it('两个字段都缺失时返回空串', () => {
    expect(currentRoleName({})).toBe('')
  })
})
