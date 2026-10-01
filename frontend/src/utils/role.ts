/**
 * 用户表单「角色」下拉的选项拼装
 *
 * v0.5.0 PR-2 起角色是数据库里的数据（`GET /api/admin/roles`），不再是一份
 * 写死在前端的 [admin, user] 清单——角色管理页新建一个角色，这里立刻就能选到。
 *
 * 组件内部状态没法用 vitest 直接覆盖（仓库无 @vue/test-utils），
 * 因此值得测的部分放在这里，沿用 `utils/menu.ts` 的做法。
 */

/** `GET /api/admin/roles` 返回项中，本文件真正用到的字段 */
export interface RoleListItem {
  name: string
  description?: string | null
}

/** `n-select` 的选项形状 */
export interface RoleSelectOption {
  label: string
  value: string
}

/** 默认优先选中的角色：普通用户 */
export const DEFAULT_ROLE_NAME = 'user'

/**
 * 拼装下拉选项
 *
 * - label 拼上描述（`admin（系统管理员，拥有所有权限）`），只有名字的角色也能认出来
 * - **value 必须是后端返回的原始角色名**：后端是归一化的唯一数据源，
 *   前端不要自己 lower()/trim()，否则一旦后端规则调整就会两边错位
 * - `current` 是正在编辑的用户当前持有的角色。若它不在列表里（迁移
 *   `008` 会刻意保留归一化后撞名的历史角色），补一个显式选项——
 *   否则 `n-select` 只显示一个光秃秃的原始值，看不出这是个异常状态
 */
export function buildRoleSelectOptions(
  roles: readonly RoleListItem[],
  current?: string
): RoleSelectOption[] {
  const options = roles.map((role) => ({
    label: role.description ? `${role.name}（${role.description}）` : role.name,
    value: role.name,
  }))
  if (current && !options.some((o) => o.value === current)) {
    options.push({ label: `${current}（当前角色，已不在角色列表中）`, value: current })
  }
  return options
}

/**
 * 新建用户时的默认角色
 *
 * 优先普通用户（多数新建账号的意图），否则列表首个，都不存在则空串
 * （交给表单的必填校验报错，而不是静默选中一个角色）
 */
export function pickDefaultRole(roles: readonly RoleListItem[]): string {
  if (roles.some((r) => r.name === DEFAULT_ROLE_NAME)) return DEFAULT_ROLE_NAME
  return roles[0]?.name ?? ''
}

/**
 * 用户当前真正持有的角色名（用于回填编辑表单）
 *
 * **不能用 `UserInfo.role`**：后端那个字段是只有 admin / user 两值的展示用枚举
 * （`Role::primary_from`，非 admin 一律塌缩成 user），真实角色集合在 `roles` 里。
 * 直接拿 `role` 回填，一个持有自定义角色的用户一打开编辑框就会变成"普通用户"，
 * 一保存就把角色静默改掉了。
 *
 * 传 `roles` 的第一个；缺失时退回 `role`（兼容只回了主角色的老响应）。
 */
export function currentRoleName(user: { role?: string; roles?: readonly string[] }): string {
  return user.roles?.[0] ?? user.role ?? ''
}
