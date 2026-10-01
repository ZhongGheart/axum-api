/** 菜单管理 API */

import http from './index'

export interface MenuNode {
  id: string
  parent_id: string | null
  name: string
  path: string | null
  component: string | null
  icon: string | null
  sort_order: number
  type: string
  permission: string | null
  is_visible: boolean
  /**
   * 可恢复的权限码（仅后端返回）
   *
   * 权限码被清空后全系统就没有任何角色再持有它，而"改写权限码必须持有
   * 目标码"的守卫会把写回也一并拦死。这个字段告诉界面"这个按钮的码
   * 可以恢复"，恢复入口才不至于无从发现。为 null 表示没有可恢复的清空记录。
   */
  restorable_permission?: string | null
  children: MenuNode[]
}

export interface CreateMenuReq {
  parent_id?: string
  name: string
  path?: string
  component?: string
  icon?: string
  sort_order?: number
  type: string
  permission?: string
  is_visible?: boolean
}

export const menuApi = {
  /** GET /api/auth/menus — 当前登录用户可见的导航菜单（前端动态路由的数据源） */
  myMenus() {
    return http.get<MenuNode[]>('/auth/menus')
  },

  /** GET /api/admin/menus */
  list() {
    return http.get<MenuNode[]>('/admin/menus')
  },

  /** POST /api/admin/menus */
  create(data: CreateMenuReq) {
    return http.post<MenuNode>('/admin/menus', data)
  },

  /** PUT /api/admin/menus/:id */
  update(id: string, data: CreateMenuReq) {
    return http.put<MenuNode>(`/admin/menus/${id}`, data)
  },

  /** DELETE /api/admin/menus/:id */
  delete(id: string) {
    return http.delete<null>(`/admin/menus/${id}`)
  },

  /**
   * POST /api/admin/menus/:id/restore-permission — 恢复被清空的权限码
   *
   * 只有清空者本人能调（服务端按 user id 判定），且不要求当前持有该码。
   */
  restorePermission(id: string) {
    return http.post<MenuNode>(`/admin/menus/${id}/restore-permission`)
  },

  /**
   * GET /api/admin/menus?role_id= — 某角色已授权的菜单树
   *
   * 与 `list()` 一样返回 `type='button'` 的节点：授权树需要展示并勾选权限码按钮。
   */
  listByRole(roleId: string) {
    return http.get<MenuNode[]>('/admin/menus', { params: { role_id: roleId } })
  },
}
