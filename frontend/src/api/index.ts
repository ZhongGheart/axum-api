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
import { getToken } from '@/utils/storage'
import { showError } from '@/utils/message'
import { notifySessionEnded } from '@/utils/session'
import { ApiError } from '@/api/errors'
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

    // 二进制响应不参与缓存：缓存命中时返回的是**伪造的 response**，
    // 里面只有 `headers: {'x-cache': 'HIT'}`。导出的截断状态正是走响应头
    // 告诉前端的（`x-export-truncated`），一旦被缓存吞掉，
    // 界面就会在数据被截断时照样报"导出成功"——又变回无声失败。
    const isBinary = config.responseType === 'blob' || config.responseType === 'arraybuffer'

    // GET 请求尝试读取缓存（通过 cancelToken 机制短路）
    if (ENABLE_CACHE && config.method === 'get' && !isBinary) {
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

/**
 * 401 里**不**代表"会话被吊销"的端点
 *
 * 判据是"这个 401 有没有正常的用户语义"，而不是"是不是登录相关接口"。
 * 登出同样要排除：用户点退出登录时，是他在结束会话，不是会话被结束。
 */
const SESSION_END_EXEMPT = new Set(['/auth/login', '/auth/logout'])

/**
 * 401 时**不弹提示**的端点
 *
 * 登出的 401 对用户没有任何可行动作：`logout()` 本来就会清本地会话并跳登录页，
 * 再补一句"令牌已被注销，请重新登录"只会让人以为自己操作错了。
 */
const SILENT_ON_401 = new Set(['/auth/logout'])

/**
 * 取请求路径（去掉 baseURL 与查询串），用于上表的匹配
 *
 * 用**后缀**匹配而不是全等：axios 的 `config.url` 不含 baseURL，但调用点
 * 万一写成完整路径（`/api/auth/login`）时，全等匹配会漏掉——
 * 漏掉的后果是**输错口令被弹去"会话已失效"**，登录页从此无法正常报错。
 * 这是安全方向的失败，宁可多匹配。
 */
function requestPathOf(error: AxiosError): string {
  const url = error.config?.url || ''
  return (url.split('?')[0] || '').replace(/\/+$/, '')
}

/** 路径是否命中给定端点集合（后缀匹配，见 requestPathOf 的说明） */
function matchesPath(path: string, set: Set<string>): boolean {
  for (const p of set) {
    if (path === p || path.endsWith(p)) return true
  }
  return false
}

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

    // 写操作后失效 GET 缓存：否则列表/详情会继续返回修改前的数据
    if (response.config?.method && response.config.method !== 'get') {
      requestCache.invalidate()
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
      return Promise.reject(new ApiError('请求超时', 0, true))
    }
    if (!error.response) {
      showError('网络异常，请检查连接')
      return Promise.reject(new ApiError('网络异常', 0, true))
    }

    const status = error.response.status

    // 后端已经分得清 401 的三种原因（口令错 / 令牌被吊销 / 令牌无效过期），
    // 优先用它自己的话。前端此前对 401 无条件写死"未授权，请重新登录"，
    // 于是登录页输错口令也被告知"未授权"——说的是另一件事。
    const responseBody = error.response.data as { message?: unknown } | undefined
    const serverMessage = typeof responseBody?.message === 'string' ? responseBody.message : ''

    /*
     * 默认**用后端的话**，前端不另编通用句。
     *
     * 此前这里对 400 / 409 / 413 一律写死 `请求失败 (400)`，把后端
     * message 整段丢掉。后端是分得清的（`校验失败: 验证码不正确，请确认
     * 手机时间准确后重试` 与 `校验失败: 恢复码已被使用` 说的是两件事，
     * 用户该知道自己下一步做什么），把它换成状态码等于用"发生了什么"
     * 冒充"为什么"，和上面 401 那段踩的是同一个坑。
     */
    let message = serverMessage || `请求失败 (${status})`

    switch (status) {
      case 401:
        // 会话已失效：清空缓存，避免换账号后读到上一会话的数据
        requestCache.invalidate()

        // 登录与登出的 401 有**正常的用户语义**，不当作"会话被吊销"：
        // - 登录：口令错误，用户正站在登录页上等他改，重定向只会把人弹走
        // - 登出：点"退出登录"的人**正是**主动结束会话的人，
        //   告诉他"会话已失效"是把因果说反了
        if (!matchesPath(requestPathOf(error), SESSION_END_EXEMPT)) {
          // 用后端原话，不在前端另编通用文案
          const reason = serverMessage || '登录状态已失效，请重新登录'
          notifySessionEnded(reason)
          // 抛出的错误也要带这句话，而不是 `请求失败 (401)`：
          // 监控页等界面会直接把 error.message 显示出来（见 views/monitor/*），
          // 让"请求失败 (401)"出现在界面上，是拿状态码当解释。
          //
          // 不在此处弹窗：提示由登录页承接（跳转后弹窗一闪即逝），
          // 跳转会清掉令牌并离开当前页面，再弹一句是噪音
          return Promise.reject(new ApiError(reason, status, true))
        }
        if (matchesPath(requestPathOf(error), SILENT_ON_401)) {
          return Promise.reject(
            new ApiError(serverMessage || '登出请求未成功', status, true),
          )
        }
        message = serverMessage || '未授权，请重新登录'
        break
      case 403:
        // 无权访问的**原因**分得清（缺哪个权限码），比"权限不足"可行动
        message = serverMessage || '权限不足'
        break
      case 404:
        message = serverMessage || '请求的资源不存在'
        break
      case 429:
        // 锁定类错误带"还剩几次/解锁时间"，那正是用户要看到的
        message = serverMessage || '请求过于频繁，请稍后再试'
        break
      case 500:
        // 500 刻意不看后端消息：内部错误的具体原因按设计不外泄
        message = '服务器内部错误'
        break
    }

    showError(message)
    return Promise.reject(new ApiError(message, status, true))
  },
)

export default http
