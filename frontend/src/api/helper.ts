/**
 * Axios 辅助工具
 *
 * 请求重试逻辑 + 错误消息集成。
 */

import type { AxiosError, AxiosResponse } from 'axios'
import { showError } from '@/utils/message'

/** 重试配置 */
export interface RetryOptions {
  /** 最大重试次数 */
  maxRetries: number
  /** 重试间隔（毫秒） */
  retryDelay: number
  /** 哪些 HTTP 状态码需要重试 */
  retryStatuses: number[]
}

/** 默认重试配置 */
export const defaultRetryOptions: RetryOptions = {
  maxRetries: 2,
  retryDelay: 1000,
  retryStatuses: [408, 429, 500, 502, 503],
}

/**
 * 判断是否应重试
 */
export function shouldRetry(
  error: AxiosError,
  options: RetryOptions = defaultRetryOptions,
): boolean {
  const status = error.response?.status
  if (!status) {
    // 网络错误（无响应）→ 重试
    return true
  }
  return options.retryStatuses.includes(status)
}

/**
 * 延迟等待
 */
export function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/**
 * 统一错误处理（显示消息 + 返回值）
 */
export function handleError(error: unknown): never {
  if (error instanceof Error) {
    showError(error.message)
    throw error
  }
  const fallback = new Error('未知错误')
  showError(fallback.message)
  throw fallback
}

/**
 * 类型安全的 HTTP 响应解包（配合响应拦截器后使用）
 */
export function unwrapResponse<T>(response: unknown): T {
  return response as T
}
