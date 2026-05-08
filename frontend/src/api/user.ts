/**
 * 用户管理 API
 *
 * 对应后端 controller/user.rs 的管理员接口。
 */

import http from './index'
import type { UserInfo } from './types/response'

/** 用户管理接口 */
export const userApi = {
  /** GET /api/admin/users?page=1&page_size=10 */
  list(params: { page?: number; page_size?: number }) {
    return http.get<{
      items: UserInfo[]
      total: number
      page: number
      page_size: number
      total_pages: number
    }>('/admin/users', { params })
  },

  /** POST /api/admin/users */
  create(data: {
    username: string
    email: string
    password?: string
    role: string
    is_active?: boolean
  }) {
    return http.post<UserInfo>('/admin/users', data)
  },

  /** PUT /api/admin/users/:id */
  update(
    id: string,
    data: {
      username: string
      email: string
      role: string
      is_active?: boolean
    },
  ) {
    return http.put<UserInfo>(`/admin/users/${id}`, data)
  },

  /** DELETE /api/admin/users/:id */
  delete(id: string) {
    return http.delete<null>(`/admin/users/${id}`)
  },
}
