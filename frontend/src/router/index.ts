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
    component: () => import('@/views/login/index.vue'),
    meta: { title: '登录', layout: 'blank' },
  },
  {
    path: '/register',
    name: 'Register',
    component: () => import('@/views/register/index.vue'),
    meta: { title: '注册', layout: 'blank' },
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
// 路由守卫：鉴权拦截 + Token 过期检测
// ============================================

/** 解析 JWT payload（不验证签名，仅读取过期时间） */
function parseJwtPayload(token: string): { exp?: number } | null {
  try {
    const parts = token.split('.')
    if (parts.length !== 3) return null
    const payload = parts[1]
    const decoded = JSON.parse(atob(payload))
    return { exp: decoded.exp }
  } catch {
    return null
  }
}

/** 检查 Token 是否过期 */
function isTokenExpired(token: string): boolean {
  const claims = parseJwtPayload(token)
  if (!claims?.exp) return true
  const now = Math.floor(Date.now() / 1000)
  // 预留 30 秒缓冲，避免边缘情况
  return claims.exp - 30 <= now
}

router.beforeEach((to, _from, next) => {
  document.title = `${to.meta.title || 'Axum Admin'}`

  const token = getToken()

  // Token 存在但已过期 → 清除并重定向到登录页
  if (token && isTokenExpired(token)) {
    localStorage.clear()
    if (to.path !== '/login') {
      return next('/login')
    }
  }

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
