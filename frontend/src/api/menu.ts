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
   * GET /api/admin/menus?role_id= — 某角色已授权的菜单树
   *
   * 与 `list()` 一样返回 `type='button'` 的节点：授权树需要展示并勾选权限码按钮。
   */
  listByRole(roleId: string) {
    return http.get<MenuNode[]>('/admin/menus', { params: { role_id: roleId } })
  },
}
