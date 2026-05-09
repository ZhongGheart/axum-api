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
}
