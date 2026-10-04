/**
 * 用户管理 API
 *
 * 对应后端 controller/user.rs 的管理员接口。
 */

import http from './index'
import type {
  ImportUsersResult,
  UnlockResult,
  UserInfo,
  UserSession,
} from './types/response'

/** 用户管理接口 */
export const userApi = {
  /**
   * GET /api/admin/users?page=1&page_size=10&keyword=...
   *
   * `keyword` 同时匹配用户名与邮箱；后端对未知参数返回 400 而非静默忽略，
   * 所以这里多传一个字段会被立刻发现，而不是让筛选"看起来没反应"。
   */
  list(params: {
    page?: number
    page_size?: number
    keyword?: string
    /**
     * 激活状态筛选（v0.20.0）：`true` 只看启用，`false` 只看禁用，**不传则不限**
     *
     * 不传必须真的"不带这个参数"，不能传 `undefined` 之外的东西：
     * 后端 DTO 上有 `deny_unknown_fields`，多传会被 400 拒掉。
     * 用 `undefined` 让 axios 直接省略该 query 项。
     */
    is_active?: boolean
    /** 角色名筛选：只看拥有该角色的用户；空串等同不过滤 */
    role?: string
  }) {
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
    /** 所属部门 ID（v0.24.0）；null 表示无部门 */
    dept_id?: string | null
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
      /** 所属部门 ID（v0.24.0）；null 表示清空，不传表示不修改 */
      dept_id?: string | null
      is_active?: boolean
    },
  ) {
    return http.put<UserInfo>(`/admin/users/${id}`, data)
  },

  /** DELETE /api/admin/users/:id */
  delete(id: string) {
    return http.delete<null>(`/admin/users/${id}`)
  },

  /** POST /api/admin/users/:id/unlock — 解锁被临时锁定的账号（v0.20.0） */
  unlock(id: string) {
    return http.post<UnlockResult>(`/admin/users/${id}/unlock`)
  },

  /**
   * GET /api/admin/users/:id/sessions — 列出该用户的在线会话（v0.20.0）
   *
   * 返回的是**数组本身**而不是 `{items, total}`：这个列表没有分页
   * （一个账号的活跃令牌数就是那么几个），套一个分页信封只会
   * 让前端多写一层解包，而 `total` 恒等于 `items.length`。
   */
  sessions(id: string) {
    return http.get<UserSession[]>(`/admin/users/${id}/sessions`)
  },

  /**
   * POST /api/admin/users/:id/sessions/:jti/revoke — 吊销单个会话（v0.20.0）
   *
   * 与"吊销该用户全部会话"是两条独立路径：后者走时间戳水位，
   * 这里只让一个设备失效。
   */
  revokeSession(id: string, jti: string) {
    return http.post<null>(`/admin/users/${id}/sessions/${jti}/revoke`)
  },

  /**
   * POST /api/admin/users/import — CSV 批量导入用户（v0.20.0）
   *
   * 必需列 `username,email,password,roles`，可选列 `display_name`；
   * `roles` 单元格内用 `|` 分隔多个角色。
   *
   * `dry_run: true` 时只校验不落库。**200 不代表全部成功**——
   * 要看 `failed` 与 `failures`，界面必须逐条展示失败行。
   */
  importUsers(csv: string, dryRun = false) {
    return http.post<ImportUsersResult>('/admin/users/import', { csv, dry_run: dryRun })
  },
}
