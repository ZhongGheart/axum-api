/**
 * 路由配置
 *
 * 包含基础路由和动态加载的系统管理路由。
 * 系统管理路由需要 admin 角色才能访问（路由守卫 + 路由元信息拦截）。
 */
import { createRouter, createWebHistory } from 'vue-router'
import type { RouteRecordRaw } from 'vue-router'
import { getToken } from '@/utils/storage'

/** 白名单路由（无需登录） */
const WHITE_LIST = ['/login', '/register']

/** 基础路由表 */
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
  // ── 系统管理（仅 admin 可访问） ──────────────────────────────
  {
    path: '/system',
    name: 'System',
    redirect: '/system/user',
    meta: { title: '系统管理', roles: ['admin'] },
    children: [
      {
        path: 'user',
        name: 'SystemUser',
        component: () => import('@/views/system/user/index.vue'),
        meta: { title: '用户管理', roles: ['admin'] },
      },
      {
        path: 'role',
        name: 'SystemRole',
        component: () => import('@/views/system/role/index.vue'),
        meta: { title: '角色管理', roles: ['admin'] },
      },
    ],
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
// 路由守卫：鉴权 + Token 过期 + 角色检测
// ============================================

/** 解析 JWT payload */
function parseJwtPayload(token: string): { exp?: number; roles?: string[]; role?: string } | null {
  try {
    const parts = token.split('.')
    if (parts.length !== 3) return null
    return JSON.parse(atob(parts[1]))
  } catch {
    return null
  }
}

/** 检查 Token 是否过期 */
function isTokenExpired(token: string): boolean {
  const claims = parseJwtPayload(token)
  if (!claims?.exp) return true
  return claims.exp - 30 <= Math.floor(Date.now() / 1000)
}

router.beforeEach((to, _from, next) => {
  document.title = `${to.meta.title || 'Axum Admin'}`
  const token = getToken()

  // Token 过期检测
  if (token && isTokenExpired(token)) {
    localStorage.clear()
    return next('/login')
  }

  // 白名单放行
  if (WHITE_LIST.includes(to.path)) {
    if (token && to.path === '/login') return next('/')
    return next()
  }

  // 未登录拦截
  if (!token) return next('/login')

  // ── 角色权限检测 ───────────────────────────────────────────
  const requiredRoles = to.meta.roles as string[] | undefined
  if (requiredRoles && requiredRoles.length > 0) {
    const claims = parseJwtPayload(token)
    const userRoles = claims?.roles || (claims?.role ? [claims.role] : [])
    const hasRole = requiredRoles.some((r) => userRoles.includes(r))
    if (!hasRole) {
      return next('/')
    }
  }

  next()
})

export default router
