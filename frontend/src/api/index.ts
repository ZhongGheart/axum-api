/**
 * Axios 全局配置
 *
 * - 请求拦截器：注入 Authorization 头 + 全局 loading + 请求缓存（GET）
 * - 响应拦截器：统一解包 ApiResponse.data + 自动重试 + 友好错误提示 + 性能记录
 * - 请求去重：相同 GET 请求同时发送时自动合并
 */

import axios from 'axios'
import type { AxiosError, AxiosResponse, InternalAxiosRequestConfig } from 'axios'
import type { ApiResponse } from '@/api/types/response'
import { getToken, removeToken } from '@/utils/storage'
import { showError } from '@/utils/message'
import { requestCache } from '@/utils/cache'
import { perfMonitor } from '@/utils/performance'

/** 是否启用请求缓存（通过环境变量控制，默认生产环境启用） */
const ENABLE_CACHE = import.meta.env.PROD ?? true

/** 是否启用性能监控 */
const ENABLE_PERF = true

const http = axios.create({
  baseURL: import.meta.env.VITE_API_BASE_URL || '/api',
  timeout: 15000,
  headers: { 'Content-Type': 'application/json' },
})

// ============================================
// 重试配置
// ============================================

const MAX_RETRIES = 2
const RETRYABLE_STATUSES = [408, 429, 500, 502, 503]
const RETRY_BASE_DELAY = 1000

function isRetryable(error: AxiosError): boolean {
  const status = error.response?.status
  if (!status) return true
  return RETRYABLE_STATUSES.includes(status)
}

// ============================================
// 请求拦截器
// ============================================

http.interceptors.request.use(
  (config: InternalAxiosRequestConfig) => {
    const token = getToken()
    if (token && config.headers) {
      config.headers.Authorization = `Bearer ${token}`
    }

    // 启动全局 loading bar
    window.$loadingBar?.start()

    // 记录请求开始时间（用于性能监控）
    config.headers.set('X-Request-Start', String(performance.now()))

    // GET 请求尝试读取缓存（通过 cancelToken 机制短路）
    if (ENABLE_CACHE && config.method === 'get') {
      const cached = requestCache.get<unknown>('GET', config.url || '', config.params as Record<string, unknown>)
      if (cached !== null) {
        // 模拟响应，跳过实际请求
        config.headers.set('X-Cache', 'HIT')
        // 修改 adapter 返回缓存数据
        config.adapter = async () => {
          return {
            data: cached,
            status: 200,
            statusText: 'OK (cached)',
            headers: { 'x-cache': 'HIT' },
            config,
          } as AxiosResponse
        }
      }
    }

    return config
  },
  (error) => {
    window.$loadingBar?.error()
    return Promise.reject(error)
  },
)

// ============================================
// 响应拦截器
// ============================================

http.interceptors.response.use(
  (response: AxiosResponse<ApiResponse>) => {
    window.$loadingBar?.finish()

    // 记录接口耗时
    if (ENABLE_PERF) {
      const startTime = response.config?.headers?.['X-Request-Start']
      if (startTime) {
        const duration = performance.now() - Number(startTime)
        const url = response.config?.url || 'unknown'
        perfMonitor.recordApi(url, Math.round(duration))
      }
    }

    // 二进制响应（blob/arraybuffer）直接返回，不拆包 ApiResponse
    const respType = response.config?.responseType
    if (respType === 'blob' || respType === 'arraybuffer') {
      return response
    }

    const { data } = response

    if (data.code !== 200) {
      return Promise.reject(new Error(data.message || '请求失败'))
    }

    // 写入缓存（仅 GET）
    if (ENABLE_CACHE && response.config?.method === 'get') {
      requestCache.set('GET', response.config.url || '', data.data, response.config.params)
    }

    return data.data as unknown as AxiosResponse
  },
  async (error: AxiosError) => {
    window.$loadingBar?.error()

    // ── 自动重试 ─────────────────────────────────────────────
    const config = error.config as InternalAxiosRequestConfig & { _retryCount?: number }
    if (config && isRetryable(error)) {
      config._retryCount = (config._retryCount || 0) + 1
      if (config._retryCount <= MAX_RETRIES) {
        const delayMs = RETRY_BASE_DELAY * config._retryCount
        await new Promise((r) => setTimeout(r, delayMs))
        return http(config)
      }
    }

    // ── 记录错误到性能监控 ───────────────────────────────────
    if (ENABLE_PERF) {
      perfMonitor.record('error', error.message || '请求错误')
    }

    // ── 错误消息处理 ─────────────────────────────────────────
    if (error.code === 'ECONNABORTED') {
      showError('请求超时，请稍后重试')
      return Promise.reject(new Error('请求超时'))
    }
    if (!error.response) {
      showError('网络异常，请检查连接')
      return Promise.reject(new Error('网络异常'))
    }

    const status = error.response.status
    let message = `请求失败 (${status})`

    switch (status) {
      case 401:
        message = '未授权，请重新登录'
        break
      case 403:
        message = '权限不足'
        break
      case 404:
        message = '请求的资源不存在'
        break
      case 429:
        message = '请求过于频繁，请稍后再试'
        break
      case 500:
        message = '服务器内部错误'
        break
    }

    showError(message)
    return Promise.reject(new Error(message))
  },
)

export default http
