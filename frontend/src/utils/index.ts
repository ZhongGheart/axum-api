/**
 * 通用工具函数
 */

/** 格式化时间戳 */
export function formatTime(timestamp: string): string {
  const date = new Date(timestamp)
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

/** 从 localStorage 获取 Token */
export function getToken(): string | null {
  return localStorage.getItem('token')
}

/** 保存 Token */
export function setToken(token: string): void {
  localStorage.setItem('token', token)
}

/** 清除 Token */
export function removeToken(): void {
  localStorage.removeItem('token')
}
