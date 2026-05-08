/**
 * 本地加密存储工具
 *
 * 对存入 localStorage 的敏感数据进行 Base64 编码混淆，
 * 使用固定应用前缀避免 key 冲突。
 */

/** 存储前缀，防止多应用 key 冲突 */
const STORAGE_PREFIX = 'axum_'

/** 简单的 XOR + Base64 编码（前端层防明文泄露，非安全加密） */
function encode(value: string): string {
  return btoa(encodeURIComponent(value))
}

function decode(value: string): string {
  return decodeURIComponent(atob(value))
}

/** 存储项接口 */
interface StorageData<T> {
  value: T
  expire?: number // 过期时间戳（毫秒）
}

/**
 * 写入加密存储
 * @param key   存储键名（自动加前缀）
 * @param value 任意可 JSON 序列化的值
 * @param expireMs 过期时间（毫秒），可选
 */
export function setStorage<T>(key: string, value: T, expireMs?: number): void {
  try {
    const data: StorageData<T> = { value }
    if (expireMs) {
      data.expire = Date.now() + expireMs
    }
    const json = JSON.stringify(data)
    localStorage.setItem(STORAGE_PREFIX + key, encode(json))
  } catch (e) {
    console.error('存储写入失败:', e)
  }
}

/**
 * 读取加密存储
 * @param key  存储键名
 * @param def  默认值（可选）
 */
export function getStorage<T>(key: string, def?: T): T | undefined {
  try {
    const raw = localStorage.getItem(STORAGE_PREFIX + key)
    if (!raw) return def

    const json = decode(raw)
    const data: StorageData<T> = JSON.parse(json)

    // 检查是否过期
    if (data.expire && Date.now() > data.expire) {
      removeStorage(key)
      return def
    }

    return data.value
  } catch {
    return def
  }
}

/** 删除存储项 */
export function removeStorage(key: string): void {
  localStorage.removeItem(STORAGE_PREFIX + key)
}

/** 清空所有带前缀的存储项 */
export function clearStorage(): void {
  const keysToRemove: string[] = []
  for (let i = 0; i < localStorage.length; i++) {
    const key = localStorage.key(i)
    if (key && key.startsWith(STORAGE_PREFIX)) {
      keysToRemove.push(key)
    }
  }
  keysToRemove.forEach((k) => localStorage.removeItem(k))
}

// ──────────────────────────────────────────────
// Token 专用快捷方法
// ──────────────────────────────────────────────

const TOKEN_KEY = 'token'
const USER_KEY = 'user'

export function setToken(token: string): void {
  setStorage(TOKEN_KEY, token)
}

export function getToken(): string | undefined {
  return getStorage<string>(TOKEN_KEY)
}

export function removeToken(): void {
  removeStorage(TOKEN_KEY)
}

export function setUserInfo(user: Record<string, unknown>): void {
  setStorage(USER_KEY, user)
}

export function getUserInfo<T = Record<string, unknown>>(): T | undefined {
  return getStorage<T>(USER_KEY)
}

export function removeUserInfo(): void {
  removeStorage(USER_KEY)
}
