/**
 * 部门管理 API
 *
 * 对应后端 `controller/department.rs`。
 */

import http from './index'
import type {
  CreateDepartmentRequest,
  DepartmentFlat,
  DepartmentNode,
  DepartmentUser,
  MoveDepartmentRequest,
  UpdateDepartmentRequest,
} from './types/response'

export const departmentApi = {
  /** GET /api/admin/departments — 部门树 */
  tree(): Promise<DepartmentNode[]> {
    return http.get('/admin/departments') as unknown as Promise<DepartmentNode[]>
  },

  /** GET /api/admin/departments/flat — 扁平列表（用于下拉选择） */
  flatList(): Promise<DepartmentFlat[]> {
    return http.get('/admin/departments/flat') as unknown as Promise<DepartmentFlat[]>
  },

  /** POST /api/admin/departments — 新建部门 */
  create(data: CreateDepartmentRequest): Promise<DepartmentNode> {
    return http.post('/admin/departments', data) as unknown as Promise<DepartmentNode>
  },

  /** PUT /api/admin/departments/{id} — 修改部门 */
  update(id: string, data: UpdateDepartmentRequest): Promise<DepartmentNode> {
    return http.put(`/admin/departments/${id}`, data) as unknown as Promise<DepartmentNode>
  },

  /** POST /api/admin/departments/{id}/move — 移动部门 */
  move(id: string, data: MoveDepartmentRequest): Promise<DepartmentNode> {
    return http.post(`/admin/departments/${id}/move`, data) as unknown as Promise<DepartmentNode>
  },

  /** DELETE /api/admin/departments/{id} — 删除部门 */
  remove(id: string): Promise<string> {
    return http.delete(`/admin/departments/${id}`) as unknown as Promise<string>
  },

  /** GET /api/admin/departments/{id}/users — 该部门下的用户 */
  users(id: string): Promise<DepartmentUser[]> {
    return http.get(`/admin/departments/${id}/users`) as unknown as Promise<DepartmentUser[]>
  },
}
