/**
 * 菜单管理 API
 *
 * 对应后端 controller/menu.rs 的接口。
 */

import http from './index'

/** 菜单树节点 */
export interface MenuNode {
  id: string
  parent_id: string | null
  name: string
  path: string | null
  component: string | null
  icon: string | null
  sort_order: number
  type: 'menu' | 'button' | 'directory'
  permission: string | null
  is_visible: boolean
  created_at: string
  children: MenuNode[]
}

/** 创建/更新菜单请求 */
export interface CreateMenuReq {
  parent_id?: string
  name: string
  path?: string
  component?: string
  icon?: string
  sort_order?: number
  type: 'menu' | 'button' | 'directory'
  permission?: string
  is_visible?: boolean
}

/** 菜单管理接口 */
export const menuApi = {
  /** GET /api/admin/menus */
  list(roleId?: string) {
    const params = roleId ? { role_id: roleId } : undefined
    return http.get<MenuNode[]>('/admin/menus', { params })
  },

  /** POST /api/admin/menus */
  create(data: CreateMenuReq) {
    return http.post<MenuNode>('/admin/menus', data)
  },

  /** PUT /api/admin/menus/:id */
  update(id: string, data: Partial<CreateMenuReq>) {
    return http.put<MenuNode>(`/admin/menus/${id}`, data)
  },

  /** DELETE /api/admin/menus/:id */
  delete(id: string) {
    return http.delete<null>(`/admin/menus/${id}`)
  },

  /** PUT /api/admin/roles/:id/menus — 分配角色菜单权限 */
  assignRoleMenus(roleId: string, menuIds: string[]) {
    return http.put<null>(`/admin/roles/${roleId}/menus`, { menu_ids: menuIds })
  },
}
