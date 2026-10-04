/**
 * 用户状态管理
 *
 * 管理 Token、用户信息、登录/登出/获取用户信息等操作。
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { LoginRequest, LoginResponse, UserInfo } from '@/api/types/response'
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

  /**
   * 第一步：提交口令
   *
   * **不一定拿到令牌**（v0.25.0 起）：账号若绑了两步验证，后端在这里
   * 就停下并回一个挑战令牌，正式令牌要拿挑战令牌再换。因此这里的
   * 成功分支有两种，调用方（登录页）必须看 `requires_2fa` 决定下一步。
   *
   * 只有确实拿到令牌时才写本地状态——先把 `token` 写成空串再走分支，
   * 会让中间态被路由守卫当成"已登录"又当成"未登录"。
   */
  async function login(req: LoginRequest): Promise<LoginResponse | undefined> {
    try {
      // 口令经 HTTPS 明文提交，服务端用 Argon2 存储（不再做客户端预哈希）
      const data = (await authApi.login({
        username: req.username,
        password: req.password,
      })) as unknown as LoginResponse

      // 需要二次验证：只把挑战令牌交回调用方，本地会话状态一概不动
      if (data.requires_2fa) return data

      applyToken(data)
      // 换账号后不得复用上一会话的 GET 缓存
      requestCache.invalidate()

      return data
    } catch (error) {
      handleError(error)
    }
  }

  /**
   * 第二步：交第二道因子换正式令牌（v0.25.0）
   *
   * 成功后与口令直通的路径汇流到同一个 `applyToken`，两条路径
   * 之后的本地状态**不应该有任何差别**。
   */
  async function completeTwoFactorLogin(
    challengeToken: string,
    code: string,
  ): Promise<LoginResponse | undefined> {
    try {
      const data = (await authApi.verifyTwoFactor(challengeToken, code)) as unknown as LoginResponse
      if (data.requires_2fa || !data.token) return undefined
      applyToken(data)
      requestCache.invalidate()
      return data
    } catch (error) {
      handleError(error)
    }
  }

  /**
   * 把后端签发的令牌落到本地会话状态
   *
   * 口令直通与二次验证通过共用，避免两条路径各写一遍后**慢慢长歪**
   * （典型症状：某天加了个字段只改了一边，受限账号登录后行为不同）。
   */
  function applyToken(data: LoginResponse) {
    if (!data.token) return
    token.value = data.token
    setToken(data.token)
    isLoggedIn.value = true
    mustChangePassword.value = data.must_change_password === true
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

  /**
   * 用服务端返回的 UserInfo 覆盖本地状态
   *
   * 供改资料这类"后端已经返回了改完之后的完整信息"的场景复用。
   * 不重新 GET：`fetchUserInfo` 那一次往返拿的是同一份数据，
   * 而这里的目的恰恰是**用服务端的值**替换掉界面上乐观改过的值。
   */
  function applyUserInfo(info: UserInfo) {
    userInfo.value = info
    mustChangePassword.value = info.must_change_password === true
    setUserInfo(info as unknown as Record<string, unknown>)
  }

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
    completeTwoFactorLogin,
    logout,
    clearLocalSession,
    applyUserInfo,
    fetchUserInfo,
  }
})
