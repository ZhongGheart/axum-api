/**
 * Axios 全局配置
 *
 * - 请求拦截器：注入 Authorization 头
 * - 响应拦截器：统一解包 ApiResponse.data，业务层只处理数据
 */

import axios from 'axios'
import type { AxiosResponse, InternalAxiosRequestConfig } from 'axios'
import type { ApiResponse } from '@/api/types/response'

/** Axios 实例 */
const http = axios.create({
  baseURL: import.meta.env.VITE_API_BASE_URL || '/api',
  timeout: 15000,
  headers: {
    'Content-Type': 'application/json',
  },
})

// ============================================
// 请求拦截器
// ============================================

http.interceptors.request.use(
  (config: InternalAxiosRequestConfig) => {
    // 从 localStorage 获取 Token
    const token = localStorage.getItem('token')
    if (token && config.headers) {
      config.headers.Authorization = `Bearer ${token}`
    }
    return config
  },
  (error) => {
    return Promise.reject(error)
  },
)

// ============================================
// 响应拦截器
// ============================================

http.interceptors.response.use(
  (response: AxiosResponse<ApiResponse>) => {
    const { data } = response

    // 后端返回统一格式 { code, message, data }
    // code !== 200 视为业务错误
    if (data.code !== 200) {
      // 401 未授权 → 清除 Token 跳转登录
      if (data.code === 401) {
        localStorage.removeItem('token')
        window.location.href = '/login'
      }
      return Promise.reject(new Error(data.message || '请求失败'))
    }

    // 成功：直接返回 data.data（业务代码只关心数据）
    return data.data as unknown as AxiosResponse
  },
  (error) => {
    // 网络错误 / 超时
    if (error.code === 'ECONNABORTED') {
      return Promise.reject(new Error('请求超时'))
    }
    if (!error.response) {
      return Promise.reject(new Error('网络异常，请检查连接'))
    }

    const status = error.response.status
    switch (status) {
      case 401:
        localStorage.removeItem('token')
        window.location.href = '/login'
        return Promise.reject(new Error('未授权，请重新登录'))
      case 403:
        return Promise.reject(new Error('权限不足'))
      case 404:
        return Promise.reject(new Error('请求的资源不存在'))
      case 429:
        return Promise.reject(new Error('请求过于频繁，请稍后再试'))
      case 500:
        return Promise.reject(new Error('服务器内部错误'))
      default:
        return Promise.reject(new Error(`请求失败 (${status})`))
    }
  },
)

export default http
