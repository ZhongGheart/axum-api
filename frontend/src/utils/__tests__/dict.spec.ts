/**
 * 「刷新字典缓存」提示文案的单元测试（v0.16.0）
 *
 * 这个按钮此前的提示是无条件的一句"缓存刷新成功"——而后端一个键都没删。
 * 现在端点如实返回三个数字，文案要跟着如实转述。风险点有两个：
 * 1. 清了 0 个键时不能报成"已清空"（那还是在骗人，只是换了措辞）；
 * 2. "跳过已禁用类型"平时是 0，不该每点一次都冒出一句噪声。
 */
import { describe, it, expect } from 'vitest'
import { buildRefreshMessage } from '@/utils/dict'

describe('buildRefreshMessage', () => {
  it('清了 0 个键时不报"已清空"——那正是按钮什么都没做的样子', () => {
    const msg = buildRefreshMessage({
      cleared_keys: 0,
      reloaded_types: 1,
      skipped_disabled_types: 0,
    })
    expect(msg).toContain('没有需要清理的缓存键')
    expect(msg).not.toContain('已清空')
  })

  it('真的清了键才说"已清空"，并带上真实数字', () => {
    const msg = buildRefreshMessage({
      cleared_keys: 3,
      reloaded_types: 2,
      skipped_disabled_types: 0,
    })
    expect(msg).toContain('已清空 3 个缓存键')
    expect(msg).toContain('回填 2 个类型')
  })

  it('跳过数大于 0 时才提已禁用类型', () => {
    const msg = buildRefreshMessage({
      cleared_keys: 2,
      reloaded_types: 1,
      skipped_disabled_types: 1,
    })
    expect(msg).toContain('跳过 1 个已禁用类型')
  })

  it('没有已禁用类型时不加那句，避免每次点击都多一行噪声', () => {
    const msg = buildRefreshMessage({
      cleared_keys: 1,
      reloaded_types: 1,
      skipped_disabled_types: 0,
    })
    expect(msg).not.toContain('跳过')
  })
})
