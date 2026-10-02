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
import { usePermissionsStore } from '@/stores/permissions'
import { useUserStore } from '@/stores/user'
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
    children: [
      // 个人中心：静态注册，**不走后端菜单**
      //
      // 菜单是按角色授权的，而个人中心（改密）对**所有**登录用户都该存在。
      // 塞进菜单表的话，角色没勾这一项的用户就会连改密入口都没有——
      // 而"改不了密码"正是管理员重置口令想解决的问题。
      {
        path: 'profile',
        name: 'Profile',
        component: () => import('@/views/profile/index.vue'),
        meta: { title: '个人中心' },
      },
    ],
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

  // ── 受限令牌：强制改密前不放行任何业务页面 ──────────────────
  //
  // 后端已经拦住了（非 /profile 类接口一律 403），这里只是不让人
  // 先进去再撞一屏报错。用户可以自由离开个人中心去登出。
  //
  // **必须在这里就 return，不能只是改写 to.path**：
  // 受限令牌除了 /profile 什么都调不了，菜单与权限码必然 403。
  // 继续往下走去加载它们，只会换来一屏"权限不足"提示——
  // 既没有信息量，又把真正值得看的错误淹掉。
  const userStore = useUserStore()
  if (userStore.mustChangePassword) {
    return to.path === '/profile' ? next() : next('/profile')
  }

  // ── 加载菜单与权限码并注册动态路由 ─────────────────────────
  // 首次进入（含刷新）时业务路由尚未注册，必须先加载菜单再重新匹配当前地址，
  // 否则会先落到 404 匹配结果上。
  // 权限码与菜单同时加载：两者都是首屏渲染的前提（侧栏 + 按钮级显隐），
  // 且权限码加载失败时 fail-closed（按钮全藏），不阻塞路由放行。
  const menuStore = useMenuStore()
  const permissionsStore = usePermissionsStore()
  if (!menuStore.loaded) {
    try {
      const [menus] = await Promise.all([menuStore.load(), permissionsStore.load()])
      registerMenuRoutes(menus)
    } catch {
      // 加载失败已由 store 弹出提示；这里放行，由 404 页面兜底，避免守卫死循环
      return next()
    }
    return next({ ...to, replace: true })
  }

  // 菜单已加载但权限码尚未加载（例如权限码接口单独失败后刷新）
  if (!permissionsStore.loaded) {
    await permissionsStore.load()
  }

  next()
})

export default router
