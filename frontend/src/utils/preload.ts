/**
 * 资源预加载工具
 *
 * 在浏览器空闲时间预加载关键资源：路由组件、字体、图片。
 * 基于 requestIdleCallback 或 link rel=prefetch。
 */

/** 预加载 URL 列表 */
const preloadQueue: string[] = []

/** 是否已初始化 */
let initialized = false

/**
 * 添加预加载 URL
 */
export function addPreloadUrl(url: string) {
  preloadQueue.push(url)
  if (!initialized) initPreload()
}

/**
 * 批量添加预加载 URL
 */
export function addPreloadUrls(urls: string[]) {
  preloadQueue.push(...urls)
  if (!initialized) initPreload()
}

/**
 * 初始化预加载
 */
function initPreload() {
  initialized = true

  // 浏览器空闲时预加载
  if ('requestIdleCallback' in window) {
    requestIdleCallback(
      () => {
        for (const url of preloadQueue.splice(0, 10)) {
          preload(url)
        }
        // 递归处理剩余队列
        if (preloadQueue.length > 0) {
          requestIdleCallback(() => initPreload())
        }
      },
      { timeout: 5000 },
    )
  } else {
    // 降级：setTimeout
    setTimeout(() => {
      for (const url of preloadQueue.splice(0, 5)) {
        preload(url)
      }
    }, 2000)
  }
}

/**
 * 使用 <link rel="prefetch"> 预加载
 */
function preload(url: string) {
  const link = document.createElement('link')
  link.rel = 'prefetch'
  link.href = url
  document.head.appendChild(link)
}

/**
 * 预加载路由组件（基于路由表的 import 路径）
 *
 * 在登录成功后的空闲时间调用来加速页面切换。
 */
import type { RouteRecordRaw } from 'vue-router'

export function preloadRouteComponents(routes: RouteRecordRaw[]) {
  const componentPaths: string[] = []

  function collectComponents(route: RouteRecordRaw) {
    if (route.component && typeof route.component === 'function') {
      componentPaths.push(route.component.toString())
    }
    if (route.children) {
      route.children.forEach(collectComponents)
    }
  }

  routes.forEach(collectComponents)

  // 只预加载非首屏的路由组件
  addPreloadUrls(componentPaths)
}
