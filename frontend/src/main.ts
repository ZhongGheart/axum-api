/**
 * 应用入口
 *
 * - 注册 Pinia、Router、Naive UI 全局组件、自定义指令
 * - 初始化性能监控
 * - 注册全局 Api（loading bar、消息等）
 */
import { createApp } from 'vue'
import naive from 'naive-ui'
import App from './App.vue'
import router from './router'
import pinia from './stores'
import { vPermission } from './directives/permission'
import { initPerformanceMonitor } from './utils/performance'
import './assets/styles/global.css'

const app = createApp(App)

app.use(pinia)
app.use(router)
app.use(naive)

// 注册全局权限指令 v-permission
app.directive('permission', vPermission)

// 初始化性能监控（仅生产环境或显式开启时上报）
const perfEnabled = import.meta.env.VITE_PERF_MONITOR === 'true' || import.meta.env.DEV
if (perfEnabled) {
  initPerformanceMonitor()
}

// 在路由就绪后预加载非首屏路由组件（利用浏览器空闲时间）
import { preloadComponents } from './utils/lazyLoad'
router.isReady().then(() => {
  if ('requestIdleCallback' in window) {
    requestIdleCallback(
      () => {
        // 预加载系统管理相关的页面组件
        const adminRoutes = router.getRoutes().filter((r) => {
          const roles = r.meta?.roles as string[] | undefined
          return roles?.includes('admin')
        })
        const loaders = adminRoutes
          .map((r) => r.components?.default)
          .filter((c): c is () => Promise<{ default: unknown }> => typeof c === 'function')
        preloadComponents(loaders).catch(() => {})
      },
      { timeout: 3000 },
    )
  }
})

app.mount('#app')
