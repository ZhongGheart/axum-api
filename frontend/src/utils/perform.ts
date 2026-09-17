/**
 * 增强的防抖与节流工具
 *
 * 支持前缘/后缘执行、异步函数、取消、立即执行、最大等待时间。
 */

// ── 防抖 ──────────────────────────────────────────────────────

interface DebounceOptions {
  /** 前缘执行（立即执行一次，然后开始防抖） */
  leading?: boolean
  /** 后缘执行（最后一次触发后 delay 毫秒执行） */
  trailing?: boolean
  /** 最大等待时间（毫秒），超过此时间强制执行 */
  maxWait?: number
}

/**
 * 增强防抖
 *
 * 支持 leading/trailing/maxWait 三个选项。
 *
 * 使用方式：
 *   const save = useDebounce(saveData, 500, { leading: true, maxWait: 3000 })
 */
export function useDebounce<T extends (...args: Parameters<T>) => ReturnType<T>>(
  fn: T,
  delay = 300,
  options: DebounceOptions = {},
): (...args: Parameters<T>) => ReturnType<T> | undefined {
  const { leading = false, trailing = true, maxWait } = options
  let timer: ReturnType<typeof setTimeout> | null = null
  let lastInvokeTime = 0
  let result: ReturnType<T> | undefined

  return function (this: unknown, ...args: Parameters<T>): ReturnType<T> | undefined {
    const context = this
    const now = Date.now()

    // 前缘执行：首次立即触发
    if (leading && !timer) {
      result = fn.apply(context, args)
      lastInvokeTime = now
    }

    // 最大等待时间检查
    if (maxWait && now - lastInvokeTime >= maxWait) {
      if (timer) {
        clearTimeout(timer)
        timer = null
      }
      result = fn.apply(context, args)
      lastInvokeTime = now
      return result
    }

    if (timer) clearTimeout(timer)

    if (trailing) {
      timer = setTimeout(() => {
        result = fn.apply(context, args)
        timer = null
        lastInvokeTime = Date.now()
      }, delay)
    }

    return result
  }
}

// ── 节流 ──────────────────────────────────────────────────────

interface ThrottleOptions {
  /** 前缘执行 */
  leading?: boolean
  /** 后缘执行 */
  trailing?: boolean
}

/**
 * 增强节流
 *
 * 支持 leading/trailing 选项。
 *
 * 使用方式：
 *   const scroll = useThrottle(handleScroll, 100, { trailing: true })
 */
export function useThrottle<T extends (...args: Parameters<T>) => ReturnType<T>>(
  fn: T,
  delay = 300,
  options: ThrottleOptions = {},
): (...args: Parameters<T>) => ReturnType<T> | undefined {
  const { leading = true, trailing = false } = options
  let timer: ReturnType<typeof setTimeout> | null = null
  let lastTime = 0
  let result: ReturnType<T> | undefined

  return function (this: unknown, ...args: Parameters<T>): ReturnType<T> | undefined {
    const context = this
    const now = Date.now()

    if (!lastTime && leading === false) lastTime = now

    const remaining = delay - (now - lastTime)

    if (remaining <= 0) {
      if (timer) {
        clearTimeout(timer)
        timer = null
      }
      result = fn.apply(context, args)
      lastTime = now
    } else if (!timer && trailing) {
      timer = setTimeout(() => {
        result = fn.apply(context, args)
        lastTime = Date.now()
        timer = null
      }, remaining)
    }

    return result
  }
}

// ── 向后兼容的别名 ────────────────────────────────────────────

/** @deprecated 请使用 useDebounce */
export const debounce = useDebounce
/** @deprecated 请使用 useThrottle */
export const throttle = useThrottle

/** 可取消延迟（保留原接口） */
export function delay(ms: number): { promise: Promise<void>; cancel: () => void } {
  let timer: ReturnType<typeof setTimeout> | null = null
  const promise = new Promise<void>((resolve) => {
    timer = setTimeout(resolve, ms)
  })
  return {
    promise,
    cancel: () => {
      if (timer) clearTimeout(timer)
    },
  }
}

// ── 异步防抖（可用于搜索建议等场景） ─────────────────────────

/**
 * 异步防抖：每次调用都返回 Promise，最终只 resolve 最后一次的结果
 */
export function debounceAsync<T extends (...args: unknown[]) => Promise<unknown>>(
  fn: T,
  delay = 300,
): (...args: Parameters<T>) => Promise<unknown> {
  let timer: ReturnType<typeof setTimeout> | null = null
  let resolveList: Array<(value: unknown) => void> = []

  return function (...args: Parameters<T>): Promise<unknown> {
    return new Promise((resolve) => {
      resolveList.push(resolve)
      if (timer) clearTimeout(timer)
      timer = setTimeout(async () => {
        const result = await fn(...args)
        resolveList.forEach((r) => r(result))
        resolveList = []
      }, delay)
    })
  }
}
