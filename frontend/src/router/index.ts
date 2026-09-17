/**
 * 路由配置
 *
 * 包含基础路由和动态加载的系统管理路由。
 * 系统管理路由需要 admin 角色才能访问（路由守卫 + 路由元信息拦截）。
 */
import { createRouter, createWebHistory } from 'vue-router'
import type { RouteRecordRaw } from 'vue-router'
import { getToken } from '@/utils/storage'
import { useMenuStore } from '@/stores/menu'
import { buildRoutesFromMenus } from './menuRoutes'

/** 白名单路由（无需登录 + 无侧栏） */
const WHITE_LIST = ['/login', '/register']

/** 基础路由表 */
const routes: RouteRecordRaw[] = [
  // ── 白名单路由（BlankLayout） ──────────────────────────────
  {
    path: '/login',
    name: 'Login',
    component: () => import('@/layouts/BlankLayout.vue'),
    children: [
      {
        path: '',
        component: () => import('@/views/login/index.vue'),
        meta: { title: '登录' },
      },
    ],
  },
  {
    path: '/register',
    name: 'Register',
    component: () => import('@/layouts/BlankLayout.vue'),
    children: [
      {
        path: '',
        component: () => import('@/views/register/index.vue'),
        meta: { title: '注册' },
      },
    ],
  },
  {
    path: '/:pathMatch(.*)*',
    name: 'NotFound',
    component: () => import('@/layouts/BlankLayout.vue'),
    children: [
      {
        path: '',
        component: () => import('@/views/error/NotFound.vue'),
        meta: { title: '404 页面未找到' },
      },
    ],
  },

  // ── 需要登录的布局壳（业务页面由后端菜单动态注册） ──────────
  // 具体页面路由在登录后由 registerMenuRoutes() 依据 /api/auth/menus 注册，
  // 菜单增删不再需要改前端路由表。
  {
    path: '/',
    name: 'Root',
    component: () => import('@/layouts/MainLayout.vue'),
    children: [],
  },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
  scrollBehavior: () => ({ top: 0 }),
})

// ============================================
// 动态菜单路由
// ============================================

/** 已注册动态路由的移除函数，登出/换账号时统一撤销 */
const removeDynamicRoutes: Array<() => void> = []

/** 按菜单树注册业务路由（登录后调用，可重复调用） */
export function registerMenuRoutes(menus: Parameters<typeof buildRoutesFromMenus>[0]): void {
  resetDynamicRoutes()
  for (const route of buildRoutesFromMenus(menus)) {
    removeDynamicRoutes.push(router.addRoute('Root', route))
  }
}

/** 撤销全部动态菜单路由 */
export function resetDynamicRoutes(): void {
  while (removeDynamicRoutes.length > 0) {
    removeDynamicRoutes.pop()?.()
  }
}

// ============================================
// 路由守卫：鉴权 + Token 过期 + 菜单加载
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

router.beforeEach(async (to, _from, next) => {
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

  // ── 加载菜单并注册动态路由 ─────────────────────────────────
  // 首次进入（含刷新）时业务路由尚未注册，必须先加载菜单再重新匹配当前地址，
  // 否则会先落到 404 匹配结果上。
  const menuStore = useMenuStore()
  if (!menuStore.loaded) {
    try {
      const menus = await menuStore.load()
      registerMenuRoutes(menus)
    } catch {
      // 加载失败已由 store 弹出提示；这里放行，由 404 页面兜底，避免守卫死循环
      return next()
    }
    return next({ ...to, replace: true })
  }

  next()
})

export default router
