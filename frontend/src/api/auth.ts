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
  RevokedOthers,
  RevokedSession,
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
}
