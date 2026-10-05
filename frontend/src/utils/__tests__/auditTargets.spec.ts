import { describe, it, expect } from 'vitest'
import {
  changeTypeColor,
  changeTypeLabel,
  describeTarget,
  describeTargets,
  targetTypeLabel,
  TARGET_TYPE_OPTIONS,
} from '../auditTargets'
import type { AuditLogTarget } from '@/api/audit'

function t(over: Partial<AuditLogTarget> = {}): AuditLogTarget {
  return {
    target_type: 'role',
    target_id: '11111111-2222-3333-4444-555555555555',
    target_key: null,
    change_type: 'revoke',
    target_label: '运营主管',
    ...over,
  }
}

describe('targetTypeLabel', () => {
  it('给已知类型中文名', () => {
    expect(targetTypeLabel('dict_type')).toBe('字典类型')
    expect(targetTypeLabel('user_two_factor')).toBe('两步验证')
  })

  it('未知类型回落到原值而不是吞掉或显示 undefined', () => {
    expect(targetTypeLabel('whatever')).toBe('whatever')
  })
})

describe('changeTypeLabel', () => {
  it('下线会话与撤销授权是两种说法', () => {
    expect(changeTypeLabel('revoke_session')).toBe('下线会话')
    expect(changeTypeLabel('revoke')).toBe('撤销')
    expect(changeTypeLabel('revoke_session')).not.toBe(changeTypeLabel('revoke'))
  })
})

describe('changeTypeColor', () => {
  it('删除是 error，未知是 info', () => {
    expect(changeTypeColor('delete')).toBe('error')
    expect(changeTypeColor('brand-new')).toBe('info')
  })
})

describe('describeTarget', () => {
  it('用标签拼出可读的一行', () => {
    expect(describeTarget(t())).toBe('角色 运营主管 · 撤销')
  })

  it('没有标签时回落到字符串键（系统参数）', () => {
    const s = t({
      target_type: 'setting',
      target_id: null,
      target_key: 'security.password.min_length',
      change_type: 'update',
      target_label: null,
    })
    expect(describeTarget(s)).toBe('系统参数 security.password.min_length · 修改')
  })

  it('对象已被删除、只剩 UUID 时也不说空', () => {
    const s = t({ target_label: null })
    expect(describeTarget(s)).toContain('11111111-2222-3333-4444-555555555555')
  })
})

describe('describeTargets', () => {
  it('空数组必须说明是历史行，不能显示成空白或破折号', () => {
    const s = describeTargets([])
    expect(s).toContain('早于结构化上线')
    expect(s).not.toBe('—')
    expect(s.trim()).not.toBe('')
  })

  it('undefined / null 也按历史行处理', () => {
    expect(describeTargets(undefined)).toContain('早于结构化上线')
    expect(describeTargets(null)).toContain('早于结构化上线')
  })

  it('多个 target 逐个列出，一条都不丢', () => {
    const s = describeTargets([
      t({ target_type: 'user', target_label: '张三', change_type: 'delete' }),
      t({ target_type: 'user', target_label: '李四', change_type: 'delete' }),
      t({ target_type: 'user', target_label: '王五', change_type: 'delete' }),
    ])
    expect(s).toBe('用户 张三 · 删除；用户 李四 · 删除；用户 王五 · 删除')
  })
})

describe('TARGET_TYPE_OPTIONS', () => {
  it('每个选项都有中文标签，且值不为空', () => {
    expect(TARGET_TYPE_OPTIONS.length).toBe(8)
    for (const o of TARGET_TYPE_OPTIONS) {
      expect(o.label).toBeTruthy()
      expect(o.value).toBeTruthy()
    }
  })

  it('系统参数必须在选项里：它没有 UUID，只能按 target_key 查', () => {
    expect(TARGET_TYPE_OPTIONS.map((o) => o.value)).toContain('setting')
  })
})
