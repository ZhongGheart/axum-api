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
import router from '@/router'

export const useUserStore = defineStore('user', () => {
  /** JWT 令牌 */
  const token = ref<string>(getToken() || '')

  /** 当前用户信息 */
  const userInfo = ref<UserInfo | null>(getUserInfo<UserInfo | null>() || null)

  /** 是否已登录 */
  const isLoggedIn = ref(!!token.value)

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
      // 换账号后不得复用上一会话的 GET 缓存
      requestCache.invalidate()

      return data
    } catch (error) {
      handleError(error)
    }
  }

  // ── 登出 ──────────────────────────────────────────────────────

  async function logout() {
    try {
      await authApi.logout()
    } catch {
      // 即使后端登出失败，前端也清除本地状态
    } finally {
      token.value = ''
      userInfo.value = null
      isLoggedIn.value = false
      removeToken()
      removeUserInfo()
      requestCache.invalidate()
      router.push('/login')
    }
  }

  // ── 获取用户信息 ──────────────────────────────────────────────

  async function fetchUserInfo() {
    try {
      const res = await authApi.me()
      const info = res as unknown as UserInfo
      userInfo.value = info
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
    login,
    logout,
    fetchUserInfo,
  }
})
