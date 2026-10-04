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
  UpdateProfileRequest,
  UserInfo,
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
}
