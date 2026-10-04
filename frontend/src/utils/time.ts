/**
 * 时间格式化
 *
 * 界面上"3 分钟前"比"2026-10-04 09:12:33"更容易判断新旧，
 * 但审计日志这类需要精确定位的场景仍然要给绝对时间，所以两种都提供。
 */

/** 后端返回的是 RFC3339 字符串；解析失败一律返回 null，由调用方决定回退文案 */
function toDate(value: string | null | undefined): Date | null {
  if (!value) return null
  const d = new Date(value)
  return Number.isNaN(d.getTime()) ? null : d
}

/** 绝对时间：`2026-10-04 09:12:33` */
export function formatDateTime(value: string | null | undefined, fallback = '—'): string {
  const d = toDate(value)
  if (!d) return fallback
  const pad = (n: number) => String(n).padStart(2, '0')
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ` +
    `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
  )
}

/**
 * 相对时间：`3 分钟前` / `2 天前`
 *
 * 超过 30 天直接回退到日期而不是"45 天前"——
 * 超过这个尺度的相对表述已经不如绝对日期好读了。
 */
export function formatRelativeTime(value: string | null | undefined, fallback = '—'): string {
  const d = toDate(value)
  if (!d) return fallback

  const diffSec = Math.floor((Date.now() - d.getTime()) / 1000)
  // 未来时间（服务器时钟偏移）不编造"几分钟后"，照原样显示绝对时间
  if (diffSec < 0) return formatDateTime(d.toISOString(), fallback)

  if (diffSec < 60) return '刚刚'
  const min = Math.floor(diffSec / 60)
  if (min < 60) return `${min} 分钟前`
  const hour = Math.floor(min / 60)
  if (hour < 24) return `${hour} 小时前`
  const day = Math.floor(hour / 24)
  if (day < 30) return `${day} 天前`

  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

/** 运行时长：`3 天 4 小时`（用于系统监控的 uptime） */
export function formatDuration(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) return '—'
  const total = Math.max(0, Math.floor(seconds))
  const day = Math.floor(total / 86400)
  const hour = Math.floor((total % 86400) / 3600)
  const min = Math.floor((total % 3600) / 60)

  if (day > 0) return hour > 0 ? `${day} 天 ${hour} 小时` : `${day} 天`
  if (hour > 0) return min > 0 ? `${hour} 小时 ${min} 分` : `${hour} 小时`
  return `${min} 分钟`
}
