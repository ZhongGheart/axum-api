import { createRouter, createWebHistory } from 'vue-router'
import type { RouteRecordRaw } from 'vue-router'

/** 路由表 */
const routes: RouteRecordRaw[] = [
  {
    path: '/',
    name: 'Home',
    component: () => import('@/views/home/index.vue'),
    meta: { title: '首页' },
  },
  {
    path: '/login',
    name: 'Login',
    component: () => import('@/views/home/index.vue'), // 占位，后续替换为 Login 页面
    meta: { title: '登录', layout: 'blank' },
  },
  {
    path: '/:pathMatch(.*)*',
    name: 'NotFound',
    component: () => import('@/views/error/NotFound.vue'),
    meta: { title: '404 页面未找到', layout: 'blank' },
  },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
  // 滚动行为：切换到新路由时滚动到顶部
  scrollBehavior: () => ({ top: 0 }),
})

// ============================================
// 全局路由守卫
// ============================================

router.beforeEach((to, _from, next) => {
  // 设置页面标题
  document.title = `${to.meta.title || 'Axum Admin'}`
  next()
})

export default router
