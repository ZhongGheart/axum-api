/**
 * 认证相关 API
 *
 * 对应后端 controller/auth.rs 的接口。
 */

import http from './index'
import type {
  LoginRequest,
  LoginResponse,
  RegisterRequest,
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
}
