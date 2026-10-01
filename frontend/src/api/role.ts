/**
 * 角色管理 API
 *
 * 对应后端 controller/role.rs 的管理员接口。
 */

import http from './index'

/** 角色列表项 */
export interface RoleItem {
  id: string
  name: string
  description: string | null
  created_at: string
  user_count: number
}

/** 新增/更新角色请求体（后端复用同一个 `CreateRoleReq`） */
export interface RoleReq {
  name: string
  description?: string
}

/** 角色管理接口 */
export const roleApi = {
  /** GET /api/admin/roles */
  list() {
    return http.get<RoleItem[]>('/admin/roles')
  },

  /** POST /api/admin/roles */
  create(data: RoleReq) {
    return http.post<RoleItem>('/admin/roles', data)
  },

  /** PUT /api/admin/roles/:id */
  update(id: string, data: RoleReq) {
    return http.put<RoleItem>(`/admin/roles/${id}`, data)
  },

  /**
   * DELETE /api/admin/roles/:id
   *
   * 后端会拒绝删除内置角色，以及仍被用户占用的角色（400，message 说明原因）。
   */
  delete(id: string) {
    return http.delete<null>(`/admin/roles/${id}`)
  },

  /**
   * PUT /api/admin/roles/:id/menus — 全量覆盖该角色的菜单/权限码授权
   *
   * 注意是**覆盖**而非增量：传进来的 `menuIds` 就是授权后的全部条目，
   * 未包含的既有授权会被撤销。因此调用方必须提交完整勾选集合，
   * 不能只提交"新增的几个"。
   */
  assignMenus(roleId: string, menuIds: string[]) {
    return http.put<null>(`/admin/roles/${roleId}/menus`, { menu_ids: menuIds })
  },

  /** GET /api/admin/users/:id/roles */
  getUserRoles(userId: string) {
    return http.get<string[]>(`/admin/users/${userId}/roles`)
  },

  /** POST /api/admin/users/:id/roles */
  assignRole(userId: string, roleName: string) {
    return http.post<null>(`/admin/users/${userId}/roles`, {
      user_id: userId,
      role_name: roleName,
    })
  },
}
