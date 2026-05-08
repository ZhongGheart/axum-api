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

/** 角色管理接口 */
export const roleApi = {
  /** GET /api/admin/roles */
  list() {
    return http.get<RoleItem[]>('/admin/roles')
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
