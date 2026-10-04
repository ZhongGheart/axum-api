/**
 * 路由配置
 *
 * 包含基础路由和动态加载的系统管理路由。
 * 系统管理路由需要 admin 角色才能访问（路由守卫 + 路由元信息拦截）。
 */
import { createRouter, createWebHistory } from 'vue-router'
import type { RouteRecordRaw } from 'vue-router'
import { getToken } from '@/utils/storage'
import { notifySessionEnded, registerSessionEndedHandler } from '@/utils/session'
import { isUnauthorized } from '@/api/errors'
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
      // 首页：静态注册，**不走后端菜单**
      //
      // 菜单里确实有一条 `/` 指向 `home/index`，但那条记录不能作为
      // 绝对子路径挂在同为 `/` 的 Root 之下（见 menuRoutes.ts 里的说明）：
      // 那样它永远匹配不到，登录后首页会是空白。这里用空路径子路由承载，
      // 侧栏入口仍然由后端菜单驱动，指向同一个 `/`。
      {
        path: '',
        name: 'Home',
        component: () => import('@/views/home/index.vue'),
        meta: { title: '首页' },
      },

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
  //
  // 走会话失效出口，而不是自己 `localStorage.clear()` 后跳转：
  // 1. clear() 抹掉的是**整个 localStorage**，连"记住密码"的用户名一起没了；
  // 2. 登录页需要知道"为什么被踢回来"才能给出解释；
  // 3. 多一个旁路实现，就多一处将来会漏改的地方。
  //
  // 同样 next(false) 而非 next('/login')：跳转已由会话失效出口发起，
  // 再 redirect 一次是第二次导航（多半撞上重复导航失败）。
  if (token && isTokenExpired(token)) {
    notifySessionEnded('登录状态已过期，请重新登录')
    return next(false)
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
    } catch (error) {
      // 401（会话已被吊销/过期）：响应拦截器已走统一出口——清令牌 + 跳登录页。
      // 这里中止本次导航，等那个跳转接管。
      //
      // **必须调 next(false)，不能直接 return**：本守卫是回调式的
      // （`guard.length === 3`），vue-router 只有在返回值风格下才会把返回值
      // 交给 next 处理。这里不调 next 会让整条导航永远悬着，页面卡在空白。
      //
      // 也不能落到 404 兜底：那会显示"404 页面未找到"，是**方向性相反**的诊断
      // ——会话没了不等于页面不存在，按 404 去排查会去找根本不存在的路由。
      if (isUnauthorized(error)) return next(false)
      // 其它失败（网络不通 / 500）：维持现状放行，由 404 页面兜底，避免守卫死循环
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

// ============================================
// 会话失效出口
// ============================================
//
// 跳转逻辑注册在这里，而不是让 `utils/session.ts` 直接 import 本模块：
// 那样会形成 `router → stores → api → utils/session → router` 的循环依赖。
// 与 `utils/message.ts` 的 `registerGlobalApis` 是同一套路。
registerSessionEndedHandler(() => {
  // 登出会走 clearLocalSession（不发请求，避免注定失败的往返）
  useUserStore().clearLocalSession()
  // replace 而非 push：会话失效不是一次"前进"，不该在历史里留一页死路。
  // 带了 query 也不影响——登录页从 sessionStorage 取原因，不依赖地址栏。
  void router.replace('/login').catch(() => {
    // 已经在登录页时会抛重复导航错误，属正常情况
  })
})

export default router
