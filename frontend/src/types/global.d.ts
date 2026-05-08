/**
 * 全局类型扩展
 *
 * 扩展 Window 接口，声明挂载到 window 上的全局变量。
 */

/** Naive UI LoadingBar 实例 */
interface LoadingBarApi {
  start: () => void
  finish: () => void
  error: () => void
}

interface Window {
  $loadingBar: LoadingBarApi
}
