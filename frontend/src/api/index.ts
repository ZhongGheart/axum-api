/**
 * Axios 全局配置
 *
 * - 请求拦截器：注入 Authorization 头 + 全局 loading
 * - 响应拦截器：统一解包 ApiResponse.data + 自动重试 + 友好错误提示
 * - 限流友好提示：429 时显示带倒计时的消息
 */

import axios from 'axios'
import type { AxiosError, AxiosResponse, InternalAxiosRequestConfig } from 'axios'
import type { ApiResponse } from '@/api/types/response'
import { getToken, removeToken } from '@/utils/storage'
import { showError } from '@/utils/message'

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
    const { data } = response

    if (data.code !== 200) {
      return Promise.reject(new Error(data.message || '请求失败'))
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
        // 429 已由后端限流中间件处理，前端给出明确的友好提示
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
