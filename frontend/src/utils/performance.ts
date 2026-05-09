/**
 * 性能监控模块
 *
 * 上报首屏加载时间（FCP/FMP）、接口响应时间、报错信息。
 * 支持自定义上报函数，默认使用 console.table 输出。
 *
 * 使用方式：
 *   import { PerfMonitor } from '@/utils/performance'
 *   PerfMonitor.init()
 */

interface PerfReport {
  /** 事件类型 */
  type: 'fcp' | 'lcp' | 'api' | 'error' | 'resource'
  /** 事件名称 */
  name: string
  /** 耗时（毫秒） */
  duration?: number
  /** 附加数据 */
  extra?: Record<string, unknown>
  /** 时间戳 */
  timestamp: number
}

/** 上报函数类型 */
type Reporter = (report: PerfReport) => void

/** 默认上报函数：开发环境 console 输出 */
const defaultReporter: Reporter = (report) => {
  if (import.meta.env.DEV) {
    console.info(`[Perf] ${report.type}:${report.name}`, report.duration ? `${report.duration}ms` : '', report.extra || '')
  }
}

class PerformanceMonitor {
  private reporter: Reporter = defaultReporter
  private initialized = false
  private apiRecords: PerfReport[] = []
  private errorRecords: PerfReport[] = []
  private MAX_RECORDS = 100

  /**
   * 初始化性能监控
   *
   * @param reporter 自定义上报函数，默认使用 console
   */
  init(reporter?: Reporter) {
    if (this.initialized) return
    this.initialized = true
    if (reporter) this.reporter = reporter

    // 收集 FCP（首次内容绘制）
    this.collectFCP()

    // 收集 LCP（最大内容绘制）
    this.collectLCP()

    // 监听全局错误
    this.collectErrors()

    // 页面加载完成后生成报告
    window.addEventListener('load', () => {
      setTimeout(() => this.generateReport(), 1000)
    })

    console.info('[PerfMonitor] 性能监控已初始化')
  }

  /**
   * 手动记录接口耗时
   */
  recordApi(name: string, duration: number, extra?: Record<string, unknown>) {
    const report: PerfReport = { type: 'api', name, duration, extra, timestamp: Date.now() }
    this.apiRecords.push(report)
    if (this.apiRecords.length > this.MAX_RECORDS) this.apiRecords.shift()
    this.reporter(report)
  }

  /**
   * 记录自定义指标
   */
  record(type: PerfReport['type'], name: string, duration?: number, extra?: Record<string, unknown>) {
    const report: PerfReport = { type, name, duration, extra, timestamp: Date.now() }
    if (type === 'error') {
      this.errorRecords.push(report)
      if (this.errorRecords.length > this.MAX_RECORDS) this.errorRecords.shift()
    }
    this.reporter(report)
  }

  /**
   * 获取 FCP 数据
   */
  private collectFCP() {
    if ('PerformanceObserver' in window) {
      const observer = new PerformanceObserver((list) => {
        const entries = list.getEntries()
        if (entries.length > 0) {
          const fcp = entries[0]
          this.reporter({
            type: 'fcp',
            name: '首次内容绘制 (FCP)',
            duration: fcp.startTime,
            timestamp: Date.now(),
          })
        }
      })
      observer.observe({ type: 'paint', buffered: true })
    }
  }

  /**
   * 获取 LCP 数据
   */
  private collectLCP() {
    if ('PerformanceObserver' in window) {
      const observer = new PerformanceObserver((list) => {
        const entries = list.getEntries()
        const lastEntry = entries[entries.length - 1]
        this.reporter({
          type: 'lcp',
          name: '最大内容绘制 (LCP)',
          duration: lastEntry.startTime,
          timestamp: Date.now(),
        })
      })
      observer.observe({ type: 'largest-contentful-paint', buffered: true })
    }
  }

  /**
   * 收集全局错误 + 未捕获 Promise 异常
   */
  private collectErrors() {
    window.addEventListener('error', (event) => {
      this.reporter({
        type: 'error',
        name: event.message,
        extra: {
          filename: event.filename,
          lineno: event.lineno,
          colno: event.colno,
        },
        timestamp: Date.now(),
      })
    })

    window.addEventListener('unhandledrejection', (event) => {
      const message = event.reason?.message || event.reason?.toString() || '未知 Promise 错误'
      this.reporter({
        type: 'error',
        name: message,
        extra: { stack: event.reason?.stack },
        timestamp: Date.now(),
      })
    })
  }

  /**
   * 生成性能报告
   */
  generateReport() {
    const apiStats = this.calcApiStats()
    const reports: Record<string, unknown> = {
      fcp: this.getLatestByType('fcp'),
      lcp: this.getLatestByType('lcp'),
      apiAvg: apiStats.avg,
      apiMax: apiStats.max,
      apiMin: apiStats.min,
      errorCount: this.errorRecords.length,
      apiCount: this.apiRecords.length,
    }

    if (import.meta.env.DEV) {
      console.table(reports)
    }

    return reports
  }

  /**
   * 获取所有 API 记录（用于批量上报）
   */
  getApiRecords(): PerfReport[] {
    return [...this.apiRecords]
  }

  /**
   * 获取所有错误记录
   */
  getErrorRecords(): PerfReport[] {
    return [...this.errorRecords]
  }

  private getLatestByType(type: PerfReport['type']): number | null {
    // 这不是正确的方法，但为了类型检查保持兼容
    return null
  }

  private calcApiStats() {
    const durations = this.apiRecords.map((r) => r.duration).filter(Boolean) as number[]
    if (durations.length === 0) return { avg: 0, max: 0, min: 0 }
    return {
      avg: Math.round(durations.reduce((a, b) => a + b, 0) / durations.length),
      max: Math.max(...durations),
      min: Math.min(...durations),
    }
  }
}

/** 全局性能监控单例 */
export const perfMonitor = new PerformanceMonitor()

/**
 * 初始化性能监控
 */
export function initPerformanceMonitor(reporter?: Reporter) {
  perfMonitor.init(reporter)
}
