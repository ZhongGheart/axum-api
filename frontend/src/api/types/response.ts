/**
 * 全局通用类型定义
 *
 * 复刻后端 Rust 统一返回结构体 ApiResponse<T>
 */

/** 后端统一返回结构体 */
export interface ApiResponse<T = unknown> {
  code: number
  message: string
  data: T | null
}

/** 登录请求 */
export interface LoginRequest {
  username: string
  password: string
}

/** 登录响应 */
export interface LoginResponse {
  token: string
  token_type: string
}

/** 注册请求 */
export interface RegisterRequest {
  username: string
  email: string
  password: string
}

/** 用户信息（不包含密码） */
export interface UserInfo {
  id: string
  username: string
  email: string
  role: 'admin' | 'user'
  is_active: boolean
  created_at: string
}

/** 分页请求参数 */
export interface PageParams {
  page: number
  page_size: number
}

/** 分页响应数据 */
export interface PageResult<T> {
  items: T[]
  total: number
  page: number
  page_size: number
  total_pages: number
}
