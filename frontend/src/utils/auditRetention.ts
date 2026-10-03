/**
 * 审计日志保留策略的**纯判定与文案**（v0.14.0）
 *
 * 为什么从 `.vue` 里抽出来：保留天数怎么措辞、什么时候该提示"你筛的区间
 * 可能已被清理"，都是**会骗人**的判定。把它们放在 SFC 里就只能靠 e2e
 * 截图去验，而截图看不出"这句话在边界情况下是不是错的"。
 * 抽成纯函数后，两类边界都能被单测钉住。
 *
 * 全部函数都**不改输入**，且对 `null` / 非法输入有明确约定——
 * 宁可少说一句，不可说错一句。
 */

import type { AuditRetentionInfo } from '@/api/audit'

/** 时刻格式化；`null` / 非法输入统一回落到破折号，不显示 `Invalid Date` */
export function formatTime(iso: string | null | undefined): string {
  if (!iso) return '—'
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return '—'
  const pad = (n: number) => String(n).padStart(2, '0')
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ` +
    `${pad(d.getHours())}:${pad(d.getMinutes())}`
  )
}

/**
 * 保留天数 + 现存最老一条的时刻
 *
 * 三种情形分别说清，不合并成一句含糊的话：
 * - 关闭自动清理 → "不自动清理"，并说明这是运维自己在管。
 *   **不能**显示成"保留 0 天"：那读起来像"日志随时都会被清光"，
 *   而实际含义是"根本不会自动清"
 * - 启用且有日志 → 给出保留天数与现存最早一条，二者共同界定了可查区间
 * - 表为空 → 明说"当前没有任何日志"，不拿"最早一条"糊弄
 */
export function describeRetention(r: AuditRetentionInfo | null): string {
  if (!r) return ''
  const window = r.oldest_log_at
    ? `现存最早一条是 ${formatTime(r.oldest_log_at)}`
    : '当前没有任何日志'
  if (!r.enabled) {
    return `日志不自动清理（保留天数设为 0，由运维自行处理）。${window}`
  }
  return `日志保留 ${r.retention_days} 天，每 ${r.cleanup_interval_seconds} 秒检查一次。${window}`
}

/**
 * 最近一次清理的说明；没清理过返回空串
 *
 * `hit_batch_limit` 必须说出来：为 true 表示**仍有更早的过期数据留在库里**，
 * 只报"删了 N 条"会让管理员以为已经清干净了。
 */
export function describePurge(r: AuditRetentionInfo | null): string {
  const p = r?.latest_purge
  if (!p) return ''
  const hit = p.hit_batch_limit
    ? '，且达到单轮上限提前收手，仍有更早的过期数据待下一轮清理'
    : ''
  return (
    `最近一次清理：${formatTime(p.ran_at)} 删除 ${p.deleted_rows} 条` +
    `（早于 ${formatTime(p.cutoff_at)} 的数据${hit}）`
  )
}

/**
 * 所选范围是否**早于**现存最老一条
 *
 * 只在真的会遮住数据时为 true：
 * - 没有保留策略信息 → false（不靠猜来打扰用户）
 * - 没选时间范围 → false
 * - 范围起点晚于最老一条 → false（那段时间的数据本来就查得到）
 * - 表为空 → false（"没有任何日志"已在主文案里说明）
 */
export function rangeStartsBeforeOldest(
  range: [number, number] | null,
  oldestLogAt: string | null,
): boolean {
  if (!range || !oldestLogAt) return false
  const oldest = new Date(oldestLogAt).getTime()
  const from = new Date(range[0]).getTime()
  if (Number.isNaN(oldest) || Number.isNaN(from)) return false
  return from < oldest
}
