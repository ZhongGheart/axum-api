/**
 * 组件懒加载工具
 *
 * 基于 defineAsyncComponent + Suspense 的懒加载包装器，
 * 支持加载中、错误、超时三种状态。
 *
 * 使用方式：
 *   const HeavyTable = lazyLoad(() => import('@/components/HeavyTable.vue'))
 *   // 等同于
 *   const HeavyTable = defineAsyncComponent({ loader: () => import('@/components/HeavyTable.vue'), ... })
 */

import { defineAsyncComponent, h } from 'vue'
import { NSpin } from 'naive-ui'

/** 默认 loading 组件 */
const DefaultLoading = {
  render() {
    return h(
      'div',
      { style: 'display:flex;align-items:center;justify-content:center;min-height:200px' },
      h(NSpin, { size: 'medium' }),
    )
  },
}

/** 默认 error 组件（使用函数式组件避免 this 引用问题） */
function DefaultError(props: { error?: Error }) {
  return h(
    'div',
    { style: 'text-align:center;padding:40px;color:#999' },
    ['组件加载失败', props.error ? `: ${props.error.message}` : ''],
  )
}
DefaultError.props = { error: { type: Error, default: null } }

interface LazyLoadOptions {
  /** 加载超时（毫秒），默认 30s */
  timeout?: number
  /** 加载中组件 */
  loadingComponent?: ReturnType<typeof defineAsyncComponent>
  /** 错误组件 */
  errorComponent?: ReturnType<typeof defineAsyncComponent>
  /** 延迟显示 loading（毫秒），防止闪烁 */
  delay?: number
  /** 是否可重试 */
  retryable?: boolean
}

/**
 * 创建懒加载组件
 *
 * @param loader  - 动态导入函数，如 () => import('@/views/xxx.vue')
 * @param options - 配置项
 * @returns 懒加载组件
 */
export function lazyLoad(
  loader: () => Promise<{ default: unknown }>,
  options: LazyLoadOptions = {},
) {
  const {
    timeout = 30000,
    loadingComponent = DefaultLoading,
    errorComponent = DefaultError,
    delay = 200,
    retryable = true,
  } = options

  return defineAsyncComponent({
    loader,
    loadingComponent,
    errorComponent,
    delay,
    timeout,
    onError(error, retry, fail, attempts) {
      if (retryable && attempts <= 3) {
        console.warn(`[lazyLoad] 组件加载失败，重试第 ${attempts} 次:`, error)
        retry()
      } else {
        console.error(`[lazyLoad] 组件加载失败 (${attempts} 次):`, error)
        fail()
      }
    },
  })
}

/**
 * 预加载组件（不渲染，仅触发下载）
 * 在空闲时间或特定时机手动调用
 */
export function preloadComponent(loader: () => Promise<unknown>) {
  const start = performance.now()
  return loader().then(() => {
    const elapsed = (performance.now() - start).toFixed(0)
    console.info(`[preload] 组件预加载完成 (${elapsed}ms)`)
  })
}

/**
 * 批量预加载组件
 */
export function preloadComponents(loaders: (() => Promise<unknown>)[]) {
  return Promise.all(loaders.map(preloadComponent))
}
