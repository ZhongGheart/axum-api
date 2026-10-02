/**
 * 全局通用类型定义
 *
 * 复刻后端 Rust 统一返回结构体 ApiResponse<T>
 * 全量类型对齐后端 model/*.rs 实体。
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
  /**
   * 令牌是否为"受限令牌"（用户须先改初始密码）
   *
   * 真正的拦截在后端 auth_middleware；前端据此跳转只是体验，
   * 不能当作安全边界。
   */
  must_change_password: boolean
}

/** 注册请求 */
export interface RegisterRequest {
  username: string
  email: string
  password: string
}

/** 用户信息（不包含密码 + 包含角色列表） */
export interface UserInfo {
  id: string
  username: string
  email: string
  role: 'admin' | 'user'
  roles?: string[]
  is_active: boolean
  /** 是否必须先改初始密码（v0.11.0） */
  must_change_password: boolean
  created_at: string
}

/** 自助修改密码请求 */
export interface ChangePasswordRequest {
  old_password: string
  new_password: string
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
