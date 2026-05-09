/** 系统监控 API */
import http from './index'

export interface SystemInfo {
  system: {
    os: string; hostname: string; kernel: string; uptime_seconds: number
    cpu: { usage_percent: number; core_count: number; frequency_mhz: number }
    memory: { total_mb: number; used_mb: number; free_mb: number; usage_percent: number }
    disks: { total_gb: number; used_gb: number; free_gb: number; usage_percent: number; name: string }[]
  }
  database: { connected: boolean; active_connections: number; max_connections: number; idle_connections: number; database_size_mb: number }
  redis: { connected: boolean; uptime_seconds: number; used_memory_bytes: number; connected_clients: number; total_commands_processed: number }
}

export interface ApiMetric {
  path: string; method: string; call_count: number; error_count: number
  total_duration_ms: number; avg_duration_ms: number; max_duration_ms: number; min_duration_ms: number
}

export interface AlertItem {
  level: string; message: string
}

export const monitorApi = {
  getSystem() { return http.get<SystemInfo>('/admin/monitor/system') },
  getApiMetrics() { return http.get<{ metrics: ApiMetric[]; summary: Record<string, unknown> }>('/admin/monitor/api-metrics') },
  getAlerts() { return http.get<{ alerts: AlertItem[]; alert_count: number }>('/admin/monitor/alerts') },
  resetMetrics() { return http.post<null>('/admin/monitor/metrics/reset') },
  exportSystem() { return http.get('/admin/monitor/system/export', { responseType: 'blob' }) },
}
