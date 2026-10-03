/**
 * 会话失效的单点出口
 *
+ *
 * 这些用例盯的是**行为契约**，不是实现细节：
 * - 同一原因只触发一次（否则并发请求会把提示刷成多条）
 * - 不同原因各自触发一次（去重不能把真实的不同原因也压掉）
 * - 原因落 sessionStorage 而非 localStorage（"记住密码"必须留在后者）
 * - 取走即清除（否则下一次正常登录仍显示旧原因）
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  notifySessionEnded,
  registerSessionEndedHandler,
  resetSessionEnded,
  takeSessionEnded,
} from '@/utils/session'

beforeEach(() => {
  sessionStorage.clear()
  resetSessionEnded()
})

afterEach(() => {
  registerSessionEndedHandler(null)
  sessionStorage.clear()
})

describe('notifySessionEnded', () => {
  it('触发注册的回调，并把后端给的原因原样传下去', () => {
    const handler = vi.fn()
    registerSessionEndedHandler(handler)

    notifySessionEnded('令牌已被注销，请重新登录')

    expect(handler).toHaveBeenCalledTimes(1)
    expect(handler).toHaveBeenCalledWith({ message: '令牌已被注销，请重新登录' })
  })

  it('同一原因在时间窗内只触发一次（并发请求不会刷出多条提示）', () => {
    const handler = vi.fn()
    registerSessionEndedHandler(handler)

    // 菜单与权限码是 Promise.all 并发，同一瞬间回来两个 401
    expect(notifySessionEnded('令牌已被注销，请重新登录', 1000)).toBe(true)
    expect(notifySessionEnded('令牌已被注销，请重新登录', 1200)).toBe(false)
    expect(notifySessionEnded('令牌已被注销，请重新登录', 1400)).toBe(false)

    expect(handler).toHaveBeenCalledTimes(1)
  })

  it('不同原因各自触发一次（去重只压重复，不压真实差异）', () => {
    const handler = vi.fn()
    registerSessionEndedHandler(handler)

    notifySessionEnded('令牌已被注销，请重新登录', 1000)
    notifySessionEnded('登录状态已失效，请重新登录', 1100)

    expect(handler).toHaveBeenCalledTimes(2)
    // 后者才是最新原因，取走时应拿到它
    expect(takeSessionEnded()).toBe('登录状态已失效，请重新登录')
  })

  it('超出时间窗后再次触发（去重不是永久静音）', () => {
    const handler = vi.fn()
    registerSessionEndedHandler(handler)

    notifySessionEnded('令牌已被注销，请重新登录', 1000)
    notifySessionEnded('令牌已被注销，请重新登录', 1000 + 2000)

    expect(handler).toHaveBeenCalledTimes(2)
  })

  it('原因落在 sessionStorage，不动 localStorage', () => {
    localStorage.setItem('axum_remember_login', '{"value":{"username":"alice"}}')
    registerSessionEndedHandler(vi.fn())

    notifySessionEnded('令牌已被注销，请重新登录')

    // 记住的用户名必须还在：会话失效不该顺手抹掉用户的输入
    expect(localStorage.getItem('axum_remember_login')).not.toBeNull()
    expect(localStorage.getItem('axum_token')).toBeNull()
    expect(takeSessionEnded()).toBe('令牌已被注销，请重新登录')
  })

  it('没有注册回调时不抛错（登录页仍能取到原因）', () => {
    registerSessionEndedHandler(null)

    expect(() => notifySessionEnded('令牌已被注销，请重新登录')).not.toThrow()
    expect(takeSessionEnded()).toBe('令牌已被注销，请重新登录')
  })
})

describe('takeSessionEnded', () => {
  it('取走即清除：下一次正常登录不应显示旧原因', () => {
    registerSessionEndedHandler(vi.fn())
    notifySessionEnded('令牌已被注销，请重新登录')

    expect(takeSessionEnded()).toBe('令牌已被注销，请重新登录')
    expect(takeSessionEnded()).toBeNull()
  })

  it('没有会话结束时返回 null（提示不是常驻的）', () => {
    expect(takeSessionEnded()).toBeNull()
  })
})

describe('registerSessionEndedHandler', () => {
  it('重新注册会复位去重状态（避免上一次运行的去重影响本次）', () => {
    const first = vi.fn()
    const second = vi.fn()
    registerSessionEndedHandler(first)
    notifySessionEnded('令牌已被注销，请重新登录', 1000)

    registerSessionEndedHandler(second)
    notifySessionEnded('令牌已被注销，请重新登录', 1100)

    expect(first).toHaveBeenCalledTimes(1)
    expect(second).toHaveBeenCalledTimes(1)
  })
})
