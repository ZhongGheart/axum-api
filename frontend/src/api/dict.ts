/** 数据字典 API */

import http from './index'

/** 字典类型 */
export interface DictTypeItem {
  id: string
  code: string
  name: string
  description: string | null
  status: string
  sort_order: number
  created_at: string
}

/** 字典项记录 */
export interface DictItemRecord {
  id: string
  dict_type_id: string
  label: string
  value: string
  sort_order: number
  status: string
  is_default: boolean
  color: string | null
  created_at: string
}

/** 字典项响应（缓存用） */
export interface DictItemInfo {
  id: string
  label: string
  value: string
  sort_order: number
  status: string
  is_default: boolean
  color: string | null
}

/** 创建字典类型请求 */
export interface CreateDictTypeReq {
  code: string
  name: string
  description?: string
  status?: string
  sort_order?: number
}

/** 创建字典项请求 */
export interface CreateDictItemReq {
  dict_type_id?: string
  label: string
  value: string
  sort_order?: number
  status?: string
  is_default?: boolean
  color?: string
}

export const dictApi = {
  /** 字典类型 CRUD */
  listTypes() { return http.get<DictTypeItem[]>('/admin/dict/types') },
  createType(data: CreateDictTypeReq) { return http.post<DictTypeItem>('/admin/dict/types', data) },
  updateType(id: string, data: CreateDictTypeReq) { return http.put<DictTypeItem>(`/admin/dict/types/${id}`, data) },
  deleteType(id: string) { return http.delete<null>(`/admin/dict/types/${id}`) },

  /** 字典项 CRUD */
  listItems(dictTypeId: string) { return http.get<DictItemRecord[]>('/admin/dict/items', { params: { dict_type_id: dictTypeId } }) },
  createItem(data: CreateDictItemReq) { return http.post<DictItemRecord>('/admin/dict/items', data) },
  updateItem(id: string, data: CreateDictItemReq) { return http.put<DictItemRecord>(`/admin/dict/items/${id}`, data) },
  deleteItem(id: string) { return http.delete<null>(`/admin/dict/items/${id}`) },

  /** 字典读取（任意已登录用户；非管理页面也会用到） */
  getCachedDict(code: string) { return http.get<DictItemInfo[]>(`/dict/${code}/items`) },
  refreshCache() { return http.post<null>('/admin/dict/refresh') },
}
