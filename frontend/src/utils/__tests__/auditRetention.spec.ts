/**
 * 保留策略文案的单元测试（v0.14.0）
 *
 * 这一版的全部价值在"说得对不对"，而说得对不对**只在边界情况下**才暴露：
 * 关闭清理时显示"保留 0 天"、撞上限时不说"已清干净"、
 * 空表时拿"最早一条"糊弄——这三种都读起来通顺，但都在骗人。
 * 所以下面每条用例都盯一个具体的骗法，而不是盯"函数返回了字符串"。
 */
import { describe, it, expect } from 'vitest'
import {
  describePurge,
  describeRetention,
  formatTime,
  rangeStartsBeforeOldest,
} from '@/utils/auditRetention'
import type { AuditRetentionInfo } from '@/api/audit'

const base = (over: Partial<AuditRetentionInfo> = {}): AuditRetentionInfo => ({
  enabled: true,
  retention_days: 90,
  cleanup_interval_seconds: 3600,
  oldest_log_at: '2026-07-01T10:00:00Z',
  latest_purge: null,
  ...over,
})

describe('formatTime', () => {
  it('空值与非法输入都回落到破折号，不显示 Invalid Date', () => {
    expect(formatTime(null)).toBe('—')
    expect(formatTime(undefined)).toBe('—')
    expect(formatTime('')).toBe('—')
    expect(formatTime('not-a-date')).toBe('—')
  })
})

describe('describeRetention', () => {
  it('拿不到策略时不输出任何文案', () => {
    // 宁可不说，也不能显示一个猜出来的"保留 90 天"
    expect(describeRetention(null)).toBe('')
  })

  it('启用清理时给出保留天数与现存最早一条', () => {
    const t = describeRetention(base())
    expect(t).toContain('保留 90 天')
    expect(t).toContain('现存最早一条')
  })

  it('关闭清理时说明"不自动清理"，且不得出现"保留 0 天"', () => {
    // "保留 0 天"读起来像"日志随时都会被清光"，与实情完全相反
    const t = describeRetention(base({ enabled: false, retention_days: 0 }))
    expect(t).toContain('不自动清理')
    expect(t).not.toContain('保留 0 天')
  })

  it('表为空时明说没有任何日志，不拿"最早一条"糊弄', () => {
    const t = describeRetention(base({ oldest_log_at: null }))
    expect(t).toContain('当前没有任何日志')
    expect(t).not.toContain('现存最早一条')
  })
})

describe('describePurge', () => {
  it('没清理过时输出空串', () => {
    expect(describePurge(base())).toBe('')
    expect(describePurge(null)).toBe('')
  })

  it('正常清完时给出删除行数与截止时刻', () => {
    const t = describePurge(base({
      latest_purge: {
        cutoff_at: '2026-07-01T00:00:00Z',
        deleted_rows: 42,
        ran_at: '2026-07-01T03:00:00Z',
        duration_ms: 120,
        hit_batch_limit: false,
      },
    }))
    expect(t).toContain('删除 42 条')
    // 必须带出截止时刻，否则只说"删了 42 条"仍答不出"从哪天起查不到"
    expect(t).toContain('早于')
  })

  it('撞上单轮上限时必须说出来，否则"没清干净"会被读成"清干净了"', () => {
    const t = describePurge(base({
      latest_purge: {
        cutoff_at: '2026-07-01T00:00:00Z',
        deleted_rows: 200000,
        ran_at: '2026-07-01T03:00:00Z',
        duration_ms: 900,
        hit_batch_limit: true,
      },
    }))
    expect(t).toContain('提前收手')
    expect(t).toContain('仍有更早的过期数据')
  })
})

describe('rangeStartsBeforeOldest', () => {
  const OLDEST = '2026-07-01T00:00:00Z'
  const t = (iso: string) => new Date(iso).getTime()

  it('范围起点早于现存最早一条时才为 true', () => {
    expect(rangeStartsBeforeOldest([t('2026-06-01T00:00:00Z'), t('2026-10-01T00:00:00Z')], OLDEST))
      .toBe(true)
  })

  it('范围整体晚于现存最早一条时为 false——那段数据本来就查得到', () => {
    expect(rangeStartsBeforeOldest([t('2026-08-01T00:00:00Z'), t('2026-10-01T00:00:00Z')], OLDEST))
      .toBe(false)
  })

  it('起点正好等于最早一条时不算早于', () => {
    expect(rangeStartsBeforeOldest([t(OLDEST), t('2026-10-01T00:00:00Z')], OLDEST)).toBe(false)
  })

  it('没有范围、没有最早时刻、时刻非法时都不提示', () => {
    // 缺前提就不猜：报一个"可能已被清理"而实际没有，比不报更糟
    expect(rangeStartsBeforeOldest(null, OLDEST)).toBe(false)
    expect(rangeStartsBeforeOldest([t('2026-06-01T00:00:00Z'), t('2026-10-01T00:00:00Z')], null)).toBe(false)
    expect(rangeStartsBeforeOldest([t('2026-06-01T00:00:00Z'), t('2026-10-01T00:00:00Z')], 'bad')).toBe(false)
  })
})
