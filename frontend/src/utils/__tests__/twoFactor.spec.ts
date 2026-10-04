/**
 * 两步验证工具的单元测试（v0.25.0）
 *
 * 恢复码归一化必须与后端 `hash_recovery_code` 同一套规则：用户从纸质
 * 备份手抄回来的码能不能认，取决于两边是否一致。后端那边有测试守着
 * 自己那半，这里守前端这半——两边各测一次，改一边才不会被漏掉。
 */
import { describe, it, expect } from 'vitest'
import {
  groupSecret,
  isRecoveryCodeFilled,
  normalizeRecoveryCode,
  formatRecoveryCodes,
  renderQrDataUrl,
} from '@/utils/twoFactor'

describe('normalizeRecoveryCode', () => {
  it('大小写与分隔符不影响结果', () => {
    expect(normalizeRecoveryCode('abcd-efgh ij')).toBe('ABCDEFGHIJ')
    expect(normalizeRecoveryCode('  ABCDEFGHIJ  ')).toBe('ABCDEFGHIJ')
  })

  it('去掉全部空白，包括换行与制表符', () => {
    expect(normalizeRecoveryCode('ABCD\nEFGH')).toBe('ABCDEFGH')
    expect(normalizeRecoveryCode('ABCD\tEFGH')).toBe('ABCDEFGH')
  })

  it('已归一化的码再归一化不改变结果（幂等）', () => {
    const once = normalizeRecoveryCode('k3mq-7xpt')
    expect(normalizeRecoveryCode(once)).toBe(once)
  })
})

describe('isRecoveryCodeFilled', () => {
  it('少填一半时拦下，明显完整时放行', () => {
    expect(isRecoveryCodeFilled('abcd')).toBe(false)
    expect(isRecoveryCodeFilled('abcd-efgh')).toBe(true)
  })

  it('不按固定长度判定', () => {
    expect(isRecoveryCodeFilled('ABCDEFGHIJKLMNOP')).toBe(true)
  })
})

describe('groupSecret', () => {
  it('每 4 位一组，末组可不足 4 位', () => {
    expect(groupSecret('ABCDEFGHIJ')).toBe('ABCD EFGH IJ')
  })

  it('已含空格时不产生双空格', () => {
    expect(groupSecret('ABCD EFGH')).toBe('ABCD EFGH')
    expect(groupSecret('ABCD EFGH').replace(/\s/g, '')).toBe('ABCDEFGH')
  })
})

describe('formatRecoveryCodes', () => {
  it('每行一个码', () => {
    expect(formatRecoveryCodes(['ABCD2345', 'EFGH6789'])).toBe('ABCD2345\nEFGH6789')
  })
})

describe('renderQrDataUrl', () => {
  it('返回可直接塞进 img src 的 PNG Data URL', async () => {
    const url = await renderQrDataUrl('otpauth://totp/Axum:alice?secret=ABCDEFGHIJKLMNOP')
    expect(url.startsWith('data:image/png;base64,')).toBe(true)
    expect(url.length).toBeGreaterThan(100)
  })
})
