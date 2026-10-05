/** 审计日志 API */

import http from './index'
import type { PageResult } from './types/response'

/** 被操作对象的一类（v0.26.0） */
export type AuditTargetType =
  | 'user'
  | 'role'
  | 'menu'
  | 'dict_type'
  | 'dict_item'
  | 'department'
  | 'setting'
  | 'user_two_factor'

/** 变更类型（v0.26.0） */
export type AuditChangeType =
  | 'create'
  | 'update'
  | 'delete'
  | 'grant'
  | 'revoke'
  | 'enable'
  | 'disable'
  | 'status'
  | 'revoke_session'
  | 'login'

/**
 * 一次操作涉及的**每一个**对象（v0.26.0）
 *
 * 是数组而不是单个字段：批量删除一次碰 N 个用户、删除角色连带撤销 N 个权限码。
 * 存成单值就只能记下第一个，其余静默丢失——那会让"另外两个被谁删了"永远查不到，
 * 而且没有任何报错。
 *
 * `target_id` 与 `target_key` 至少有一个非空：UUID 资源用 `target_id`，
 * 字符串主键的资源（如系统参数 `security.password.min_length`）用 `target_key`。
 * 后端**故意不加外键**，所以对象被删掉后 target 仍在，`target_label` 留得住名字。
 */
export interface AuditLogTarget {
  target_type: AuditTargetType
  target_id: string | null
  target_key: string | null
  change_type: AuditChangeType
  /** 对象的可读名字。对象已删除时仍保留，因此能答出"删掉的是什么" */
  target_label: string | null
}

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
  /**
   * 涉及的对象列表
   *
   * **空数组不代表"这次没改任何东西"**：v0.26.0 上线前的历史行一律是空的，
   * 界面上要把这两种情况分开说，否则管理员会以为那段时间的操作没被记录。
   */
  targets: AuditLogTarget[]
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
  /** 对象类型，精确匹配。后端对看不懂的值返回 400 而不是"查不到" */
  target_type?: string
  /** 对象 UUID，精确匹配。批量操作涉及的每个对象都能单独查出来 */
  target_id?: string
  /** 对象字符串键（如系统参数名），精确匹配。与 `target_id` 二选一 */
  target_key?: string
}

/**
 * 保留策略的一轮清理（v0.14.0）
 *
 * 此前这个信息只存在于服务端 stdout 的一行 `tracing::info!`：
 * 界面查不到、接口查不到。于是"日志从某天起就查不到了"与
 * "那天什么都没发生过"在管理员眼里完全一样——这个歧义本身就是审计的失效。
 */
export interface AuditPurgeInfo {
  /** 本轮删掉的行都早于该时刻 */
  cutoff_at: string
  deleted_rows: number
  ran_at: string
  duration_ms: number | null
  /**
   * 是否因达到单轮批数上限而提前收手
   *
   * 为 true 表示**仍有过期行留在库里**，下一轮才会继续删。
   * 不报这个区别，"还有更多过期数据没清"就会被当成"已经清干净了"。
   */
  hit_batch_limit: boolean
}

/** 审计日志保留策略（v0.14.0） */
export interface AuditRetentionInfo {
  /** 是否启用自动清理 */
  enabled: boolean
  /** 保留天数；0 表示不自动清理 */
  retention_days: number
  /** 清理任务运行间隔（秒） */
  cleanup_interval_seconds: number
  /** 现存日志中最老一条的时刻；null 表示表为空 */
  oldest_log_at: string | null
  /** 最近一次清理记录；null 表示启用以来一次都没删过 */
  latest_purge: AuditPurgeInfo | null
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
   * GET /api/admin/audit-logs/retention
   *
   * 界面据此如实说明"还能查到多早的数据"，而不是让人自己撞上
   * 一个查不到任何结果的日期范围、去怀疑那天是不是真的什么都没发生。
   */
  retention(): Promise<AuditRetentionInfo> {
    return http.get('/admin/audit-logs/retention') as unknown as Promise<AuditRetentionInfo>
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
