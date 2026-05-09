/** 审计日志 API */

import http from './index'

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

export const auditApi = {
  /** GET /api/admin/audit-logs */
  list(params: { page?: number; page_size?: number; action?: string; username?: string }) {
    return http.get<AuditLogItem[]>('/admin/audit-logs', { params })
  },

  /** GET /api/admin/logs/audit/export */
  exportLogs() {
    return http.get('/admin/logs/audit/export', { responseType: 'blob' })
  },
}
