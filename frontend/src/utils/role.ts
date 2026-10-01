/**
 * 用户表单「角色」多选框的选项拼装
 *
 * v0.5.0 PR-2 起角色是数据库里的数据（`GET /api/admin/roles`），不再是一份
 * 写死在前端的 [admin, user] 清单——角色管理页新建一个角色，这里立刻就能选到。
 * v0.6.0 起这里是**多选**：数据模型（`user_roles` 表、`UserInfo.roles`）
 * 一直是多角色的，此前表单只回填 `roles[0]` 并整体覆盖提交，
 * 会把用户其余角色静默删除。
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
  current?: readonly string[]
): RoleSelectOption[] {
  const options = roles.map((role) => ({
    label: role.description ? `${role.name}（${role.description}）` : role.name,
    value: role.name,
  }))
  for (const name of current ?? []) {
    if (!options.some((o) => o.value === name)) {
      options.push({ label: `${name}（当前角色，已不在角色列表中）`, value: name })
    }
  }
  return options
}

/**
 * 新建用户时的默认角色（多选框的初始选中项）
 *
 * 优先普通用户（多数新建账号的意图），否则列表首个，都不存在则空串
 * （交给表单的必填校验报错，而不是静默选中一个角色）
 */
export function pickDefaultRoles(roles: readonly RoleListItem[]): string[] {
  if (roles.some((r) => r.name === DEFAULT_ROLE_NAME)) return [DEFAULT_ROLE_NAME]
  const first = roles[0]?.name
  return first ? [first] : []
}

/**
 * 用户当前真正持有的角色名**集合**（用于回填编辑表单与列表展示）
 *
 * **不能用 `UserInfo.role`**：后端那个字段是只有 admin / user 两值的展示用枚举
 * （`Role::primary_from`，非 admin 一律塌缩成 user），真实角色集合在 `roles` 里。
 * 直接拿 `role` 回填，一个持有自定义角色的用户一打开编辑框就会变成"普通用户"，
 * 一保存就把角色静默改掉了；只拿 `roles[0]` 同样会丢掉其余角色。
 *
 * `roles` 为空时退回 `[role]`（兼容只回了展示枚举的老响应）。
 * 注意此时退回来的是后端 `primary_from(&[])` 的塌缩值 `user`——
 * 对一个真的没有角色的用户，这不是"他持有 user"，而是"必须至少选一个"，
 * 而 `user` 是最合理的默认预选（后端也已拒绝提交空角色集合）。
 */
export function currentRoleNames(user: {
  role?: string
  roles?: readonly string[]
}): string[] {
  if (user.roles && user.roles.length > 0) return [...user.roles]
  return user.role ? [user.role] : []
}
