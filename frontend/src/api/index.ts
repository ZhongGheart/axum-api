/**
 * Axios 全局配置
 *
 * - 请求拦截器：注入 Authorization 头
 * - 响应拦截器：统一解包 ApiResponse.data，业务层只处理数据
 * - 自动重试：网络错误 / 5xx 时自动重试
 */

import axios from 'axios'
import type { AxiosError, AxiosResponse, InternalAxiosRequestConfig } from 'axios'
import type { ApiResponse } from '@/api/types/response'
import { getToken, removeToken } from '@/utils/storage'
import { showError } from '@/utils/message'

/** Axios 实例 */
const http = axios.create({
  baseURL: import.meta.env.VITE_API_BASE_URL || '/api',
  timeout: 15000,
  headers: {
    'Content-Type': 'application/json',
  },
})

// ============================================
// 请求重试计数器
// ============================================

const MAX_RETRIES = 2
const RETRYABLE_STATUSES = [408, 429, 500, 502, 503]
const RETRY_BASE_DELAY = 1000

function isRetryable(error: AxiosError): boolean {
  const status = error.response?.status
  if (!status) return true // 网络错误
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
    return config
  },
  (error) => Promise.reject(error),
)

// ============================================
// 响应拦截器
// ============================================

http.interceptors.response.use(
  (response: AxiosResponse<ApiResponse>) => {
    const { data } = response

    if (data.code !== 200) {
      if (data.code === 401) {
        removeToken()
        window.location.href = '/login'
      }
      return Promise.reject(new Error(data.message || '请求失败'))
    }

    // 成功：直接返回 data.data（业务代码只关心数据）
    return data.data as unknown as AxiosResponse
  },
  async (error: AxiosError) => {
    // ── 自动重试逻辑 ──────────────────────────────────────────
    const config = error.config as InternalAxiosRequestConfig & { _retryCount?: number }
    if (!config || !isRetryable(error)) {
      return Promise.reject(error)
    }

    config._retryCount = (config._retryCount || 0) + 1
    if (config._retryCount <= MAX_RETRIES) {
      const delayMs = RETRY_BASE_DELAY * config._retryCount
      console.info(`请求重试 (${config._retryCount}/${MAX_RETRIES}): ${config.url}, 等待 ${delayMs}ms`)
      await new Promise((resolve) => setTimeout(resolve, delayMs))
      return http(config)
    }

    // ── 错误消息处理 ──────────────────────────────────────────
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
        removeToken()
        window.location.href = '/login'
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
