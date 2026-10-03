/**
 * 会话失效提示的文案拼接
 *
 * 后端的三种原因本来就以"请重新登录"结尾（`令牌已被注销，请重新登录`）。
 * 拼接前不检查一下，登录页就会显示"请重新登录，请重新登录"。
 * 抽成纯函数是为了能直接测——挂载整个登录页来验一句文案太重。
 */
import { describe, expect, it } from 'vitest'
import { buildSessionEndedMessage } from '@/utils/session'

describe('buildSessionEndedMessage', () => {
  it('后端原话已含"请重新登录"时不再重复追加', () => {
    expect(buildSessionEndedMessage('令牌已被注销，请重新登录'))
      .toBe('令牌已被注销，请重新登录')
  })

  it('后端没提重新登录时补上（否则提示不完整）', () => {
    expect(buildSessionEndedMessage('登录状态已失效')).toBe('登录状态已失效，请重新登录')
  })

  it('逐个覆盖后端实际会返回的三种原因，都不得出现重复尾巴', () => {
    const reasons = [
      '令牌已被注销，请重新登录',
      '登录状态已失效，请重新登录',
      '令牌无效或已过期',
      '缺少 Authorization 请求头',
    ]
    for (const r of reasons) {
      const out = buildSessionEndedMessage(r)
      const occurrences = out.split('请重新登录').length - 1
      expect(occurrences, `${r} -> ${out}`).toBeLessThanOrEqual(1)
    }
  })

  it('保留后端原话（不得改写成前端自己的通用句）', () => {
    expect(buildSessionEndedMessage('令牌已被注销，请重新登录'))
      .not.toContain('会话')
  })

  it('没有原因时不产出文案（提示不是常驻的）', () => {
    expect(buildSessionEndedMessage(null)).toBe('')
  })
})
