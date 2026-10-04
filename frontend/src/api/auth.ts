/**
 * 认证相关 API
 *
 * 对应后端 controller/auth.rs 的接口。
 */

import http from './index'
import type {
  AvatarUploadResult,
  ChangePasswordRequest,
  LoginRequest,
  LoginResponse,
  RegisterRequest,
  RecoveryCodesResponse,
  RevokedOthers,
  RevokedSession,
  TwoFactorSetup,
  TwoFactorStatus,
  UpdateProfileRequest,
  UserInfo,
  UserSession,
} from './types/response'

export const authApi = {
  /** POST /api/auth/login */
  login(data: LoginRequest) {
    return http.post<LoginResponse>('/auth/login', data)
  },

  /** POST /api/auth/register */
  register(data: RegisterRequest) {
    return http.post<UserInfo>('/auth/register', data)
  },

  /** GET /api/auth/me */
  me() {
    return http.get<UserInfo>('/auth/me')
  },

  /** POST /api/auth/logout */
  logout() {
    return http.post<null>('/auth/logout')
  },

  /** GET /api/auth/permissions — 当前用户的权限码（与后端 PermissionGuard 同源） */
  myPermissions() {
    return http.get<string[]>('/auth/permissions')
  },

  /**
   * PUT /api/auth/password — 自助修改密码
   *
   * 改密成功后该用户**全部会话失效**（含本端），需重新登录。
   */
  changePassword(data: ChangePasswordRequest) {
    return http.put<null>('/auth/password', data)
  },

  /** PUT /api/auth/profile — 自助修改展示名（v0.20.0） */
  updateProfile(data: UpdateProfileRequest) {
    return http.put<UserInfo>('/auth/profile', data)
  },

  /**
   * POST /api/auth/profile/avatar — 上传头像（v0.20.0）
   *
   * 用 `multipart/form-data` 且字段名必须是 `file`。
   * **必须让浏览器自己设 Content-Type**：手写 multipart 边界会让
   * 后端解析不到任何字段，于是报"缺少 file 字段"。
   */
  uploadAvatar(file: File) {
    const form = new FormData()
    form.append('file', file)
    return http.post<AvatarUploadResult>('/auth/profile/avatar', form, {
      headers: { 'Content-Type': 'multipart/form-data' },
    })
  },

  /**
   * GET /api/auth/sessions — 列出**当前用户自己**的在线会话（v0.23.0）
   *
   * 与管理端 `/api/admin/users/{id}/sessions` 的差别只在数据来源：
   * 那里由路径里的 id 决定看谁，这里恒为调用者自己。
   */
  mySessions() {
    return http.get<UserSession[]>('/auth/sessions')
  },

  /**
   * POST /api/auth/sessions/{jti}/revoke — 吊销自己的单个会话（v0.23.0）
   *
   * **允许吊销当前会话**（管理端刻意禁止）：这里用户是主动点
   * "下线这台设备"，紧接着的 401 正是他想要的结果。
   */
  revokeMySession(jti: string) {
    return http.post<RevokedSession>(`/auth/sessions/${jti}/revoke`)
  },

  /**
   * POST /api/auth/sessions/revoke-others — 吊销除当前外的全部会话（v0.23.0）
   *
   * "账号可能被盗用"时最该有的一台开关：改密会吊销全部会话（含本端），
   * 而"只踢掉其他设备、让我继续用"在语义上更准确。
   */
  revokeMyOtherSessions() {
    return http.post<RevokedOthers>('/auth/sessions/revoke-others')
  },

  // ── 两步验证（v0.25.0）──────────────────────────────────────

  /**
   * GET /api/auth/2fa — 当前用户的两步验证状态
   */
  twoFactorStatus() {
    return http.get<TwoFactorStatus>('/auth/2fa')
  },

  /**
   * POST /api/auth/2fa/setup — 生成密钥并返回扫码 URI
   *
   * **这一步不生效**：密钥只放在 Redis 待确认槽位里（15 分钟 TTL），
   * 必须再调 enable 交一个 App 生成的码回来才落库。
   */
  setupTwoFactor() {
    return http.post<TwoFactorSetup>('/auth/2fa/setup')
  },

  /**
   * POST /api/auth/2fa/enable — 用 App 生成的码确认启用
   *
   * 返回的恢复码**明文只此一次**，之后不可再取。
   */
  enableTwoFactor(code: string) {
    return http.post<RecoveryCodesResponse>('/auth/2fa/enable', { code })
  },

  /**
   * POST /api/auth/2fa/disable — 关闭两步验证
   *
   * 要求出示**当前口令**：登录态本身可能就来自被盗设备，
   * 只凭"已登录"就能关掉 2FA，会让这功能在最需要它的场景下形同虚设。
   */
  disableTwoFactor(password: string) {
    return http.post<string>('/auth/2fa/disable', { password })
  },

  /**
   * POST /api/auth/2fa/recovery-codes — 重新生成一批恢复码（旧码立即作废）
   */
  regenerateRecoveryCodes() {
    return http.post<RecoveryCodesResponse>('/auth/2fa/recovery-codes')
  },

  /**
   * POST /api/auth/2fa/verify — 登录第二步：交第二道因子换正式令牌
   *
   * 唯一的公开 2FA 端点：它持有挑战令牌，而挑战令牌本身就代表
   * "口令已校验通过"，所以不需要再带 Authorization 头。
   */
  verifyTwoFactor(challengeToken: string, code: string) {
    return http.post<LoginResponse>('/auth/2fa/verify', {
      challenge_token: challengeToken,
      code,
    })
  },
}
