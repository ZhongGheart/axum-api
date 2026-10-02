/**
 * 角色管理 API
 *
 * 对应后端 controller/role.rs 的管理员接口。
 */

import http from './index'
import type { PageResult } from './types/response'

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
  /**
   * GET /api/admin/roles?page=1&page_size=10
   *
   * **v0.10.0 起返回分页对象**（`{ items, total, page, page_size, total_pages }`），
   * 不再是裸数组。字段名与后端 `RoleListParams` 逐字对应；
   * 后端对未知参数返回 400 而非静默忽略。
   */
  list(params: { page?: number; page_size?: number }): Promise<PageResult<RoleItem>> {
    return http.get('/admin/roles', { params }) as unknown as Promise<PageResult<RoleItem>>
  },

  /**
   * 翻页取回**全部**角色
   *
   * 存在的原因：`list()` 分页之后，用户表单里的角色下拉如果只取一页，
   * 就会**静默少显示**后面那些角色——用户看不到自己实际持有的角色，
   * 保存时可能把权限改掉。这是分页引入的新坑，必须显式堵上。
   *
   * 下拉框天然需要完整集合（没法"翻页选角色"），
   * 所以这里按后端上限 200/页 逐页取完；总数异常时抛错而不是悄悄截断。
   */
  async listAll(): Promise<RoleItem[]> {
    const pageSize = 200
    const all: RoleItem[] = []
    let page = 1
    for (;;) {
      const res = await roleApi.list({ page, page_size: pageSize })
      all.push(...res.items)
      if (all.length >= res.total) return all
      if (res.items.length === 0) {
        throw new Error(`角色列表分页异常：已取 ${all.length} 条但 total 为 ${res.total}`)
      }
      page += 1
    }
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
