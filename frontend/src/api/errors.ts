/**
 * 接口错误类型
 *
 * 此前响应拦截器 `reject(new Error(message))`，抛出的错误里**没有任何信息**：
 * 调用方既拿不到状态码，也分不清"这条提示是否已经弹过了"。
 * 后者直接导致同一个失败被弹两次——拦截器弹一次，
 * store 里的 `handleError` 再弹一次。
 */

export class ApiError extends Error {
  /** HTTP 状态码；网络层失败（无响应）为 0 */
  readonly status: number

  /** 提示是否已由响应拦截器展示过 */
  reported: boolean

  constructor(message: string, status: number, reported = false) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.reported = reported
  }
}

/** 是否是 401（会话失效）——守卫据此与网络/500 区分开 */
export function isUnauthorized(error: unknown): boolean {
  return error instanceof ApiError && error.status === 401
}
