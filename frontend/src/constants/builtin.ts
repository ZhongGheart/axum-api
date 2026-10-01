/**
 * 内置角色名（与后端 `src/model/role.rs` 的 `BUILTIN_ROLES` 一一对应）
 *
 * 后端只拒绝删除这些角色，且角色种子仅在 `roles` 表为空时写入——
 * 删掉后不会重建，系统将**永久**失去该角色。
 *
 * 前端据此不给内置角色渲染删除按钮：一个必然返回 400 的按钮没有价值。
 * 由 `src/__tests__/builtinRoles.spec.ts` 对着后端定义表做契约校验，
 * 避免两份清单各自漂移。
 */

export const ADMIN_ROLE_NAME = 'admin'

export const BUILTIN_ROLE_NAMES: readonly string[] = [ADMIN_ROLE_NAME, 'user']

/** 是否为不可删除的内置角色 */
export function isBuiltinRole(name: string): boolean {
  return BUILTIN_ROLE_NAMES.includes(name)
}
