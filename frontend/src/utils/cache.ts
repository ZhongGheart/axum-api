/**
 * 请求缓存 + 去重 + 批量合并工具
 *
 * - 缓存: 对 GET 请求结果进行内存缓存，可配置 TTL
 * - 去重: 同一时刻相同请求自动合并，避免重复发送
 * - 批量: 短时间内的同类 POST 请求可合并为一次批量请求
 */

interface CacheEntry<T> {
  data: T
  timestamp: number
  ttl: number
}

interface RequestKey {
  url: string
  method: string
  paramsKey: string // JSON 序列化的参数指纹
}

/** 请求缓存管理器 */
class RequestCache {
  private cache = new Map<string, CacheEntry<unknown>>()
  private pending = new Map<string, Promise<unknown>>()

  /** 默认 TTL: 30 秒 */
  private static DEFAULT_TTL = 30_000

  /**
   * 获取缓存的请求结果
   */
  get<T>(method: string, url: string, params?: Record<string, unknown>): T | null {
    if (method !== 'GET') return null
    const key = this.buildKey({ url, method, paramsKey: JSON.stringify(params || {}) })
    const entry = this.cache.get(key)
    if (!entry) return null
    if (Date.now() - entry.timestamp > entry.ttl) {
      this.cache.delete(key)
      return null
    }
    return entry.data as T
  }

  /**
   * 设置缓存
   */
  set<T>(method: string, url: string, data: T, params?: Record<string, unknown>, ttl = RequestCache.DEFAULT_TTL) {
    if (method !== 'GET') return
    const key = this.buildKey({ url, method, paramsKey: JSON.stringify(params || {}) })
    this.cache.set(key, { data, timestamp: Date.now(), ttl })
  }

  /**
   * 请求去重：相同请求正在发送时，复用同一个 Promise
   */
  dedup<T>(method: string, url: string, fetcher: () => Promise<T>, params?: Record<string, unknown>): Promise<T> {
    const key = this.buildKey({ url, method, paramsKey: JSON.stringify(params || {}) })

    if (this.pending.has(key)) {
      return this.pending.get(key) as Promise<T>
    }

    const promise = fetcher().finally(() => {
      this.pending.delete(key)
    })
    this.pending.set(key, promise)
    return promise
  }

  /**
   * 失效指定 URL 的缓存
   */
  invalidate(urlPattern?: RegExp) {
    if (!urlPattern) {
      this.cache.clear()
      return
    }
    for (const key of this.cache.keys()) {
      if (urlPattern.test(key)) {
        this.cache.delete(key)
      }
    }
  }

  /**
   * 清理过期缓存
   */
  clean() {
    const now = Date.now()
    for (const [key, entry] of this.cache.entries()) {
      if (now - entry.timestamp > entry.ttl) {
        this.cache.delete(key)
      }
    }
  }

  private buildKey(req: RequestKey): string {
    return `${req.method}:${req.url}:${req.paramsKey}`
  }
}

/** 全局请求缓存单例 */
export const requestCache = new RequestCache()

// 定期清理过期缓存（每分钟）
setInterval(() => requestCache.clean(), 60_000)

// ──────────────────────────────────────────────
// 批量请求合并
// ──────────────────────────────────────────────

type BatchItem<T, P> = {
  params: P
  resolve: (value: T) => void
  reject: (error: unknown) => void
}

interface BatchConfig {
  /** 批量窗口时间（毫秒） */
  windowMs: number
  /** 最大批量大小 */
  maxSize: number
}

/**
 * 批量请求合并器
 *
 * 将短时间内的同类请求合并为一次批量请求。
 *
 * 使用方式：
 *   const batcher = new BatchRequester<User[], string[]>(
 *     async (ids) => userApi.batchGet(ids),
 *     { windowMs: 50, maxSize: 50 },
 *   )
 *   batcher.add('id1').then(user => ...)
 *   batcher.add('id2').then(user => ...)  // 合并为一个请求
 */
export class BatchRequester<T, P = unknown> {
  private queue: BatchItem<T, P>[] = []
  private timer: ReturnType<typeof setTimeout> | null = null
  private readonly batchFn: (params: P[]) => Promise<T[]>
  private readonly config: BatchConfig

  constructor(batchFn: (params: P[]) => Promise<T[]>, config?: Partial<BatchConfig>) {
    this.batchFn = batchFn
    this.config = { windowMs: 50, maxSize: 50, ...config }
  }

  add(param: P): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      this.queue.push({ params: param, resolve, reject })

      if (this.queue.length >= this.config.maxSize) {
        this.flush()
      } else if (!this.timer) {
        this.timer = setTimeout(() => this.flush(), this.config.windowMs)
      }
    })
  }

  private async flush() {
    if (this.timer) {
      clearTimeout(this.timer)
      this.timer = null
    }
    const items = this.queue.splice(0, this.config.maxSize)
    if (items.length === 0) return

    const params = items.map((i) => i.params)
    try {
      const results = await this.batchFn(params)
      items.forEach((item, index) => {
        item.resolve(results[index])
      })
    } catch (error) {
      items.forEach((item) => item.reject(error))
    }
  }

  destroy() {
    if (this.timer) clearTimeout(this.timer)
    this.queue = []
  }
}
