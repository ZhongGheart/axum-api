/**
 * 权限状态管理
 *
 * 管理动态可访问路由表，根据用户角色过滤。
 */
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import type { RouteRecordRaw } from 'vue-router'
import { useUserStore } from './user'

/** 需要特定角色的路由配置 */
export interface RoleRoute {
  path: string
  roles: string[]
}

/** 默认角色-路由映射 */
const DEFAULT_ROLE_ROUTES: RoleRoute[] = [
  { path: '/system/user', roles: ['admin'] },
  { path: '/system/role', roles: ['admin'] },
]

export const usePermissionStore = defineStore('permission', () => {
  /** 完整的动态路由表 */
  const dynamicRoutes = ref<RouteRecordRaw[]>([])

  /** 当前用户可访问的路由路径集合 */
  const accessiblePaths = computed(() => {
    const userStore = useUserStore()
    if (!userStore.userInfo) return new Set<string>()

    const userRoles = (userStore.userInfo as unknown as { roles?: string[] }).roles || [
      (userStore.userInfo as unknown as { role: string }).role,
    ]

    return new Set(
      DEFAULT_ROLE_ROUTES.filter((r) => r.roles.some((role) => userRoles.includes(role))).map(
        (r) => r.path,
      ),
    )
  })

  /** 检查是否可访问某路径 */
  function hasAccess(path: string): boolean {
    return accessiblePaths.value.has(path)
  }

  return {
    dynamicRoutes,
    accessiblePaths,
    hasAccess,
  }
})
