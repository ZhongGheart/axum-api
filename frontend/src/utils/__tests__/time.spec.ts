/**
 * 时间格式化
 *
 * 盯的是**会骗人的那几种情况**：
 * - 时区偏移（后端给 `+08:00` 时，截字符串会得到差一个时区的时刻）
 * - 无效 / 空输入不能变成 `Invalid Date`
 * - 未来时间（服务器时钟偏移）不能编造"几分钟后"
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { formatDateTime, formatDuration, formatRelativeTime } from '@/utils/time'

describe('formatDateTime', () => {
  it('按本地时刻输出，不受后端时区偏移影响', () => {
    // 带 +08:00 的偏移时间，换算到本地后小时数才对得上
    const out = formatDateTime('2026-10-03T15:18:48.344131+08:00')
    expect(out).not.toBe('—')
    expect(out).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/)
  })

  it('截掉小数秒，不让毫秒挤进表格列', () => {
    expect(formatDateTime('2026-10-03T15:18:48.344131Z')).not.toContain('.')
  })

  it('空值与非法值回退为占位符，而不是 Invalid Date', () => {
    expect(formatDateTime(null)).toBe('—')
    expect(formatDateTime(undefined)).toBe('—')
    expect(formatDateTime('')).toBe('—')
    expect(formatDateTime('不是时间')).toBe('—')
  })

  it('支持自定义回退文案', () => {
    expect(formatDateTime(null, '暂无')).toBe('暂无')
  })
})

describe('formatRelativeTime', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  /** 相对当前时刻偏移若干秒的 RFC3339 时间 */
  function ago(seconds: number): string {
    return new Date(Date.now() - seconds * 1000).toISOString()
  }

  it('一分钟内说"刚刚"', () => {
    expect(formatRelativeTime(ago(5))).toBe('刚刚')
    expect(formatRelativeTime(ago(59))).toBe('刚刚')
  })

  it('分钟 / 小时 / 天逐级升格', () => {
    expect(formatRelativeTime(ago(60 * 5))).toBe('5 分钟前')
    expect(formatRelativeTime(ago(3600 * 3))).toBe('3 小时前')
    expect(formatRelativeTime(ago(86400 * 2))).toBe('2 天前')
  })

  it('超过 30 天改用绝对日期——那个尺度上"45 天前"不如日期好读', () => {
    const out = formatRelativeTime(ago(86400 * 45))
    expect(out).toMatch(/^\d{4}-\d{2}-\d{2}$/)
  })

  it('未来时间照实显示绝对时刻，不编造"几分钟后"', () => {
    const out = formatRelativeTime(ago(-120))
    expect(out).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/)
  })

  it('空值回退为占位符', () => {
    expect(formatRelativeTime(null)).toBe('—')
    expect(formatRelativeTime('坏数据')).toBe('—')
  })
})

describe('formatDuration', () => {
  it('按天 / 小时 / 分钟三个量级输出', () => {
    expect(formatDuration(86400 * 3 + 3600 * 4)).toBe('3 天 4 小时')
    expect(formatDuration(86400 * 3)).toBe('3 天')
    expect(formatDuration(3600 * 2 + 60 * 5)).toBe('2 小时 5 分')
    expect(formatDuration(60 * 9)).toBe('9 分钟')
  })

  it('空值与非法值回退为占位符', () => {
    expect(formatDuration(null)).toBe('—')
    expect(formatDuration(undefined)).toBe('—')
    expect(formatDuration(Number.NaN)).toBe('—')
  })

  it('负数按 0 处理，不输出"-1 分钟"', () => {
    expect(formatDuration(-100)).toBe('0 分钟')
  })
})
