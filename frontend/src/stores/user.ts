/**
 * 用户状态管理
 *
 * 管理 Token、用户信息、登录/登出/获取用户信息等操作。
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { LoginRequest, UserInfo } from '@/api/types/response'
import { authApi } from '@/api/auth'
import { handleError } from '@/api/helper'
import { getToken, removeToken, setToken, setUserInfo, removeUserInfo, getUserInfo } from '@/utils/storage'
import { requestCache } from '@/utils/cache'
import router, { resetDynamicRoutes } from '@/router'
import { useMenuStore } from './menu'
import { usePermissionsStore } from './permissions'

export const useUserStore = defineStore('user', () => {
  /** JWT 令牌 */
  const token = ref<string>(getToken() || '')

  /** 当前用户信息 */
  const userInfo = ref<UserInfo | null>(getUserInfo<UserInfo | null>() || null)

  /** 是否已登录 */
  const isLoggedIn = ref(!!token.value)

  /**
   * 当前令牌是否为"受限令牌"（管理员建号/重置口令后要求先改密）
   *
   * **仅用于前端跳转，不是安全边界**：真正的拦截在后端 `auth_middleware`，
   * 受限令牌访问业务接口一律 403。存它是为了让用户不必先撞一堵 403
   * 才知道自己该去改密。
   */
  const mustChangePassword = ref(
    getUserInfo<{ must_change_password?: boolean } | null>()?.must_change_password === true,
  )

  // ── 登录 ──────────────────────────────────────────────────────

  async function login(req: LoginRequest) {
    try {
      // 口令经 HTTPS 明文提交，服务端用 Argon2 存储（不再做客户端预哈希）
      const res = await authApi.login({
        username: req.username,
        password: req.password,
      })

      const data = res as unknown as { token: string; token_type: string }
      token.value = data.token
      setToken(data.token)
      isLoggedIn.value = true
      mustChangePassword.value =
        (data as unknown as { must_change_password?: boolean }).must_change_password === true
      // 换账号后不得复用上一会话的 GET 缓存
      requestCache.invalidate()

      return data
    } catch (error) {
      handleError(error)
    }
  }

  // ── 登出 ──────────────────────────────────────────────────────

  /**
   * 清空本地会话状态（**不发任何请求**）
   *
   * 单独拆出来是因为改密后不能用它：改密成功时后端已经吊销了该用户的
   * 全部会话，再调 `/auth/logout` 必然 401。那次往返既无意义，
   * 又会在控制台留下一条"Failed to load resource"，让真实错误更难找。
   */
  function clearLocalSession() {
    token.value = ''
    userInfo.value = null
    isLoggedIn.value = false
    // 必须清掉：否则下一个用受限令牌的账号登录后，
    // 路由守卫会把这个账号也拦在个人中心
    mustChangePassword.value = false
    removeToken()
    removeUserInfo()
    requestCache.invalidate()
    // 撤销按上一个账号注册的动态菜单路由，避免换账号后残留可访问页面
    resetDynamicRoutes()
    useMenuStore().reset()
    // 权限码同样必须清空：否则新账号会短暂沿用上一账号的按钮级权限
    usePermissionsStore().reset()
  }

  async function logout() {
    try {
      await authApi.logout()
    } catch {
      // 即使后端登出失败，前端也清除本地状态
    } finally {
      clearLocalSession()
    }
    router.push('/login')
  }

  // ── 获取用户信息 ──────────────────────────────────────────────

  async function fetchUserInfo() {
    try {
      const res = await authApi.me()
      const info = res as unknown as UserInfo
      userInfo.value = info
      mustChangePassword.value = info.must_change_password === true
      setUserInfo(info as unknown as Record<string, unknown>)
      return info
    } catch (error) {
      handleError(error)
    }
  }

  return {
    token,
    userInfo,
    isLoggedIn,
    mustChangePassword,
    login,
    logout,
    clearLocalSession,
    fetchUserInfo,
  }
})
