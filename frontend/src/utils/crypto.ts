/**
 * 前端密码加密工具
 *
 * 使用 Web Crypto API 对密码进行 SHA-256 哈希，
 * 确保登录/注册时不将明文密码通过网络传输。
 */

/**
 * 对字符串进行 SHA-256 哈希，返回小写十六进制字符串
 */
export async function sha256(input: string): Promise<string> {
  const encoder = new TextEncoder()
  const data = encoder.encode(input)
  const hashBuffer = await crypto.subtle.digest('SHA-256', data)
  const hashArray = Array.from(new Uint8Array(hashBuffer))
  return hashArray.map((b) => b.toString(16).padStart(2, '0')).join('')
}

/**
 * 密码加密传输用的便捷方法
 *
 * 前端对密码做 SHA-256 哈希后，后端再对哈希值做 Argon2 加密存储。
 * 登录时序：
 *   用户输入明文 → SHA256(明文) → HTTPS → 后端 SHA256 值 → Argon2 校验
 */
export async function hashPassword(password: string): Promise<string> {
  return sha256(password)
}
