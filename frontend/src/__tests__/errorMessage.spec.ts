/**
 * 响应拦截器的错误文案选择（v0.25.0）
 *
 * 修的缺陷：拦截器对 400 / 403 / 404 / 429 一律写死通用句，
 * 把后端 `message` 整段丢掉。症状是两步验证失败时用户只看到
 * 「请求失败 (400)」——而后端明明分得清"验证码不对"和"恢复码已用过"，
 * 这两件事用户该采取的下一步完全不同。
 *
 * 直接调 axios 实例上注册好的响应拦截器，而不是重新实现一遍选择逻辑：
 * 抄一份逻辑来测，测的就不是线上跑的那份了。
 */
import { describe, expect, it, vi } from 'vitest'

const { showError } = vi.hoisted(() => ({ showError: vi.fn() }))

vi.mock('@/utils/message', () => ({ showError, showSuccess: vi.fn() }))
vi.mock('@/utils/session', () => ({ notifySessionEnded: vi.fn() }))

import http from '@/api/index'

interface Rejected {
  message: string
  status: number
  reported: boolean
}

/** 造一个形状够用的 AxiosError，走一遍真实的响应拦截器 */
async function runInterceptor(status: number, message?: string): Promise<Rejected> {
  const handler = (
    http.interceptors.response as unknown as {
      handlers?: { rejected?: (e: unknown) => Promise<never> }[]
    }
  ).handlers?.[0]
  if (typeof handler?.rejected !== 'function') throw new Error('响应拦截器未注册')

  const error = {
    config: {
      headers: { set: vi.fn() },
      url: '/auth/2fa/verify',
      /*
       * 429 / 500 在 RETRYABLE_STATUSES 里，拦截器会真的重发请求。
       * 这里把计数预先打满跳过重试：本文件要验的是**文案怎么选**，
       * 让它顺带打一次真实网络请求只会引入与断言无关的失败。
       */
      _retryCount: 99,
    },
    code: 'ERR_BAD_REQUEST',
    message: 'Request failed',
    response: {
      status,
      data: message === undefined ? {} : { code: status, message, data: null },
    },
  }

  try {
    await handler.rejected(error)
  } catch (e) {
    return e as Rejected
  }
  throw new Error('预期被 reject')
}

describe('错误文案取后端原话', () => {
  it('400 用后端的具体消息，而不是「请求失败 (400)」', async () => {
    const e = await runInterceptor(400, '校验失败: 验证码不正确，请确认手机时间准确后重试')
    expect(e.message).toBe('校验失败: 验证码不正确，请确认手机时间准确后重试')
    expect(e.status).toBe(400)
    // 已在拦截器里弹过，调用方不该再弹一次
    expect(e.reported).toBe(true)
    expect(showError).toHaveBeenCalledWith(e.message)
  })

  it('400 同样分得清"恢复码已用过"与"验证码不对"', async () => {
    const a = await runInterceptor(400, '校验失败: 验证码不正确，请重试或使用恢复码')
    const b = await runInterceptor(400, '校验失败: 恢复码已被使用，请重新生成')
    expect(a.message).not.toBe(b.message)
  })

  it('403 / 404 / 429 都优先用后端原话', async () => {
    const forbidden = await runInterceptor(403, '权限不足: 缺少权限码 user:2fa')
    expect(forbidden.message).toContain('user:2fa')
    const missing = await runInterceptor(404, '资源不存在: 用户')
    expect(missing.message).toContain('用户')
    // 锁定类错误带"还剩几次"，那正是用户要看到的
    const locked = await runInterceptor(429, '账号已锁定，请 15 分钟后重试')
    expect(locked.message).toContain('15 分钟')
  })

  it('后端没给消息时才退回通用句', async () => {
    expect((await runInterceptor(400)).message).toBe('请求失败 (400)')
    expect((await runInterceptor(403)).message).toBe('权限不足')
  })

  /**
   * 500 刻意**不**用后端消息：内部错误的具体原因按设计不外泄
   * （后端 IntoResponse 只回"服务器内部错误"，细节留在服务端日志）。
   * 万一哪天真的带上了细节，这一行也不该把它显示到界面上。
   */
  it('500 不外泄后端细节', async () => {
    const e = await runInterceptor(500, '数据库连接失败: postgres://user:pw@host/db')
    expect(e.message).toBe('服务器内部错误')
  })
})
