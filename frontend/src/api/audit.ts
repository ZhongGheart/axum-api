/** 审计日志 API */

import http from './index'
import type { PageResult } from './types/response'

export interface AuditLogItem {
  id: string
  user_id: string | null
  username: string | null
  action: string
  method: string
  path: string
  params: string | null
  result: string | null
  status_code: number
  client_ip: string | null
  duration_ms: number | null
  created_at: string
}

/**
 * 审计日志查询参数
 *
 * 字段名必须与后端 `AuditLogQuery` 逐字对应。后端对未知参数返回 400
 * 而非静默忽略，所以这里拼错字段会立刻暴露，不会退化成"筛选没反应"。
 */
export interface AuditLogListParams {
  page?: number
  page_size?: number
  sort_by?: string
  sort_order?: 'asc' | 'desc'
  /** 用户名，模糊匹配 */
  username?: string
  /** 操作，模糊匹配（如 `POST /api/admin/users`） */
  action?: string
  /** 状态码，精确匹配 */
  status_code?: number
  /** 起始时间（含），RFC3339 */
  start_time?: string
  /** 结束时间（含），RFC3339 */
  end_time?: string
}

/** 导出结果：除了文件本身，还带回截断状态 */
export interface AuditExportResult {
  blob: Blob
  /** 本次实际导出的行数 */
  rowCount: number
  /** 是否因为超过上限而被截断 */
  truncated: boolean
  /** 单次导出的行数上限 */
  maxRows: number
}

export const auditApi = {
  /** GET /api/admin/audit-logs */
  list(params: AuditLogListParams): Promise<PageResult<AuditLogItem>> {
    // 响应拦截器已经把 ApiResponse 解包成 `data.data`，但 axios 的类型声明
    // 仍然是 AxiosResponse<T>。这里把类型和运行时对齐，
    // 免得每个调用点各写一遍 `as unknown as`——那种写法正是"类型在撒谎"的温床。
    return http.get('/admin/audit-logs', { params }) as unknown as Promise<PageResult<AuditLogItem>>
  },

  /**
   * GET /api/admin/logs/audit/export
   *
   * 导出走和列表**同一套**筛选条件。原实现不带任何参数，
   * 于是界面上筛了半天、导出的却还是全量。
   *
   * 单次导出有行数上限（日志表是唯一无限增长的表）。上限本身不是问题，
   * **静默截断才是**——所以后端把截断状态放在响应头里，这里读出来交给界面告知用户。
   */
  async exportLogs(params: Omit<AuditLogListParams, 'page' | 'page_size'>): Promise<AuditExportResult> {
    const res = await http.get<Blob>('/admin/logs/audit/export', {
      params,
      responseType: 'blob',
    })
    // blob 响应在拦截器里原样返回（不解包 ApiResponse），因此能读到响应头
    return {
      blob: res.data,
      rowCount: Number(res.headers['x-export-row-count'] ?? 0),
      truncated: res.headers['x-export-truncated'] === 'true',
      maxRows: Number(res.headers['x-export-max-rows'] ?? 0),
    }
  },
}
