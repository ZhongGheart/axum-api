import { createRouter, createWebHistory } from 'vue-router'
import type { RouteRecordRaw } from 'vue-router'
import { getToken } from '@/utils/storage'

/** 无需登录的白名单路由 */
const WHITE_LIST = ['/login', '/register']

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
    component: () => import('@/views/home/index.vue'), // 占位，后续替换
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
  scrollBehavior: () => ({ top: 0 }),
})

// ============================================
// 路由守卫：鉴权拦截
// ============================================

router.beforeEach((to, _from, next) => {
  // 设置页面标题
  document.title = `${to.meta.title || 'Axum Admin'}`

  const token = getToken()

  // 白名单路由（登录页、注册页）→ 直接放行
  if (WHITE_LIST.includes(to.path)) {
    // 已登录用户访问登录页 → 跳转首页
    if (token && to.path === '/login') {
      return next('/')
    }
    return next()
  }

  // 非白名单路由 → 检查登录态
  if (!token) {
    return next('/login')
  }

  next()
})

export default router
