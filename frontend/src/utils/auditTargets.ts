/**
 * 审计日志「涉及对象」的**纯判定与文案**（v0.26.0）
 *
 * 为什么从 `.vue` 里抽出来，理由同 `auditRetention.ts`：这一列怎么措辞、
 * 空列表怎么解释，都是**会骗人**的判定。留在 SFC 里就只能靠 e2e 截图验，
 * 而截图看不出"空列表到底该显示什么"这种边界。
 */

import type { AuditChangeType, AuditLogTarget, AuditTargetType } from '@/api/audit'

/** 对象类型的中文名；未知类型回落到原始值，不吞掉 */
const TYPE_LABELS: Record<AuditTargetType, string> = {
  user: '用户',
  role: '角色',
  menu: '菜单',
  dict_type: '字典类型',
  dict_item: '字典项',
  department: '部门',
  setting: '系统参数',
  user_two_factor: '两步验证',
}

/**
 * 变更类型的中文名
 *
 * `revoke_session` 与 `revoke` 必须分开：前者是"下线了一个登录会话"，
 * 后者是"撤销了一项授权"。合成一句话"撤销"会把两者混进同一条时间线，
 * 而它们要回答的问题完全不同。
 */
const CHANGE_LABELS: Record<AuditChangeType, string> = {
  create: '新增',
  update: '修改',
  delete: '删除',
  grant: '授予',
  revoke: '撤销',
  enable: '启用',
  disable: '停用',
  status: '状态变更',
  revoke_session: '下线会话',
  login: '登录',
}

export function targetTypeLabel(t: string): string {
  return TYPE_LABELS[t as AuditTargetType] ?? t
}

export function changeTypeLabel(c: string): string {
  return CHANGE_LABELS[c as AuditChangeType] ?? c
}

/** 筛选下拉的选项；标签用中文，值必须是后端 `TargetType::as_str` 的原样 */
export const TARGET_TYPE_OPTIONS = (
  Object.keys(TYPE_LABELS) as AuditTargetType[]
).map((v) => ({ label: TYPE_LABELS[v], value: v }))

/** 对象类型 → 变更类型 → 颜色，避免表格里一片同色标签 */
const CHANGE_TYPES: Record<AuditChangeType, 'success' | 'error' | 'warning' | 'info'> = {
  create: 'success',
  update: 'info',
  delete: 'error',
  grant: 'success',
  revoke: 'warning',
  enable: 'success',
  disable: 'warning',
  status: 'warning',
  revoke_session: 'warning',
  login: 'info',
}

export function changeTypeColor(c: string): 'success' | 'error' | 'warning' | 'info' {
  return CHANGE_TYPES[c as AuditChangeType] ?? 'info'
}

/** 单个 target 的一行文字，如 `角色 运营主管 · 撤销` */
export function describeTarget(t: AuditLogTarget): string {
  const name = t.target_label ?? t.target_key ?? t.target_id
  return `${targetTypeLabel(t.target_type)} ${name ?? '（未命名）'} · ${changeTypeLabel(t.change_type)}`
}

/**
 * 「涉及对象」整列的显示文字
 *
 * **空列表不能显示成空白或破折号**：空数组绝大多数是 v0.26.0 上线前的历史行，
 * 那里本来就只有 `result` 文本。若显示成"—"，管理员会以为那次操作没碰任何对象，
 * 或者更糟——以为这行的审计不可信。所以明说"早于结构化上线"。
 *
 * 只有一种情况空列表是"真的什么都没碰"：该操作确实没改任何持久对象
 * （清缓存、重置指标），或写操作被拒。这两种在 `result` 里都有文字说明，
 * 不足以让人误判成"漏记了"，但这里仍然只说事实、不替后端猜原因。
 */
export function describeTargets(targets: AuditLogTarget[] | undefined | null): string {
  if (!targets || targets.length === 0) return '该记录早于结构化上线，未记录涉及对象'
  return targets.map(describeTarget).join('；')
}
