/**
 * 用户管理 API
 *
 * 对应后端 controller/user.rs 的管理员接口。
 */

import http from './index'
import type { UserInfo } from './types/response'

/** 用户管理接口 */
export const userApi = {
  /**
   * GET /api/admin/users?page=1&page_size=10&keyword=...
   *
   * `keyword` 同时匹配用户名与邮箱；后端对未知参数返回 400 而非静默忽略，
   * 所以这里多传一个字段会被立刻发现，而不是让筛选"看起来没反应"。
   */
  list(params: { page?: number; page_size?: number; keyword?: string }) {
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
    /** 多角色。v0.6.0 起为权威字段（后端另有单数 `role` 兼容别名） */
    roles: string[]
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
      roles: string[]
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
