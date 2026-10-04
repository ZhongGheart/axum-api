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
  /**
   * 展示名（v0.20.0）
   *
   * **不是登录键**，与 `username` 是两件事：两个人同名完全合法。
   * `null` 表示"没设过"，界面应回退显示 `username`——
   * 这两种状态刻意可区分，"清空展示名"与"从没设过"不是同一个意思。
   */
  display_name: string | null
  /** 头像站内相对路径（v0.20.0）；`null` 表示没设过 */
  avatar_url: string | null
  created_at: string
}

/** 展示用名称：未设 `display_name` 时回退到用户名 */
export function displayLabel(user: Pick<UserInfo, 'username' | 'display_name'>): string {
  return user.display_name && user.display_name.length > 0 ? user.display_name : user.username
}

/** 自助修改资料请求（v0.20.0） */
export interface UpdateProfileRequest {
  /**
   * 不传该字段 = 不改；传 `null` 或空串 = 清空；传字符串 = 设置
   *
   * 这个三态由后端反序列化器区分，前端**不能**用 `undefined` 代替 null 表示清空：
   * `undefined` 在 `JSON.stringify` 里会被整个丢掉，后端收到的是"没这个字段"，
   * 于是界面上的"清空展示名"静默变成了"不改"。
   */
  display_name?: string | null
  avatar_url?: string | null
}

/** 头像上传响应（v0.20.0） */
export interface AvatarUploadResult {
  url: string
}

/**
 * 在线会话条目（v0.20.0）
 *
 * **不含 `username`**：这个端点已经在路径里指明了用户是谁，
 * 再在每条里重复一遍，两处一旦不一致就没有哪边是对的。
 */
export interface UserSession {
  jti: string
  client_ip: string
  /** 登录时刻（Unix 毫秒） */
  login_at_ms: number
  /** 令牌到期时刻（Unix 毫秒） */
  expires_at_ms: number
  /** 是否为发起本次查询的那个会话 */
  is_current: boolean
}

/** 单会话吊销结果（v0.20.0 管理端 / v0.23.0 自助端共用） */
export interface RevokedSession {
  jti: string
  expires_at_ms: number
  remaining_sessions: number
}

/** 「吊销除当前外的全部会话」的结果（v0.23.0） */
export interface RevokedOthers {
  /** 本次实际吊销的会话数（不含当前会话） */
  revoked_count: number
  /** 剩余会话数（恒为 1，即当前会话） */
  remaining_sessions: number
}

/** 解锁结果（v0.20.0） */
export interface UnlockResult {
  username: string
  cleared_failures: number
  scopes_cleared: number
}

/** CSV 导入中某一行的失败原因（v0.20.0） */
export interface ImportRowFailure {
  /** CSV 行号（含表头，从 1 开始） */
  line: number
  username: string
  reason: string
}

/** CSV 批量导入结果（v0.20.0） */
export interface ImportUsersResult {
  total: number
  created: number
  failed: number
  failures: ImportRowFailure[]
  created_usernames: string[]
  dry_run: boolean
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
