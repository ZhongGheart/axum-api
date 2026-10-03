/**
 * 401 的分诊与提示去重
 *
 * v0.15.0 的核心断言全在这里。会话被吊销后页面显示"404 页面未找到"、
 * 同一句提示弹 4 次、令牌留在 localStorage——这三件事都由
 * `api/index.ts` 响应拦截器这一处决定，所以测试也压在这一处。
 *
 * **判据落在可观测行为上**：哪些提示弹了、哪些没弹、跳转回调被调了几次、
 * 抛出的错误带不带状态码。不去断言内部调用顺序。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { AxiosError, AxiosHeaders } from 'axios'
import type { InternalAxiosRequestConfig } from 'axios'
import http from '@/api/index'
import { handleError } from '@/api/helper'
import { isUnauthorized } from '@/api/errors'

// mock 工厂与断言必须引用**同一个** vi.fn()：
// 工厂里再 new 一个的话，断言数的是另一个函数，永远是 0 次。
vi.mock('@/utils/message', () => ({ showError: vi.fn() }))
vi.mock('@/utils/session', () => ({ notifySessionEnded: vi.fn() }))

const { showError } = await import('@/utils/message')
const { notifySessionEnded } = await import('@/utils/session')

/** 让下一次请求以指定状态码失败，模拟后端的统一错误响应 */
function failWith(status: number, message: string, url = '/auth/menus') {
  http.defaults.adapter = async (config) => {
    const err = new AxiosError(
      'Request failed with status code ' + status,
      String(status),
      config as InternalAxiosRequestConfig,
      null,
      {
        status,
        statusText: 'x',
        data: { code: status, message, data: null },
        headers: new AxiosHeaders(),
        config: config as InternalAxiosRequestConfig,
      },
    )
    throw err
  }
  return () => http.get(url)
}

beforeEach(() => {
  vi.mocked(showError).mockClear()
  vi.mocked(notifySessionEnded).mockClear()
})

describe('401：会话被吊销', () => {
  it('走会话失效出口，并把后端原话传过去', async () => {
    const call = failWith(401, '令牌已被注销，请重新登录')

    await expect(call()).rejects.toThrow()

    expect(notifySessionEnded).toHaveBeenCalledTimes(1)
    // 用后端的话而不是前端通用句：后端分得清是哪一种
    expect(notifySessionEnded).toHaveBeenCalledWith('令牌已被注销，请重新登录')
  })

  it('不再弹"未授权，请重新登录"（提示由登录页承接，不刷屏）', async () => {
    const call = failWith(401, '令牌已被注销，请重新登录')

    await expect(call()).rejects.toThrow()

    expect(showError).not.toHaveBeenCalled()
  })

  it('抛出的错误带 401 状态码（守卫据此与网络/500 区分）', async () => {
    const call = failWith(401, '令牌已被注销，请重新登录')

    await expect(call()).rejects.toSatisfy(isUnauthorized)
  })

  it('后端没给 message 时兜底，且仍不弹窗', async () => {
    http.defaults.adapter = async (config) => {
      throw new AxiosError('boom', '401', config as InternalAxiosRequestConfig, null, {
        status: 401,
        statusText: 'Unauthorized',
        data: null,
        headers: new AxiosHeaders(),
        config: config as InternalAxiosRequestConfig,
      })
    }

    await expect(http.get('/auth/menus')).rejects.toThrow()

    expect(notifySessionEnded).toHaveBeenCalledWith('登录状态已失效，请重新登录')
    expect(showError).not.toHaveBeenCalled()
  })
})

describe('401：有正常用户语义，不当作会话被吊销', () => {
  it('登录口令错误：原地报后端的话，不跳走', async () => {
    const call = failWith(401, '凭证错误: 用户名或密码错误', '/auth/login')

    await expect(call()).rejects.toThrow()

    expect(notifySessionEnded).not.toHaveBeenCalled()
    // 关键：不能是"未授权，请重新登录"——输错口令不是会话问题
    expect(showError).toHaveBeenCalledWith('凭证错误: 用户名或密码错误')
  })

  it('登出：既不跳走也不弹提示（用户正是主动结束会话的人）', async () => {
    const call = failWith(401, '令牌已被注销，请重新登录', '/auth/logout')

    await expect(call()).rejects.toThrow()

    expect(notifySessionEnded).not.toHaveBeenCalled()
    expect(showError).not.toHaveBeenCalled()
  })

  it('请求路径带 baseURL 前缀时同样能匹配到豁免清单', async () => {
    const call = failWith(401, '凭证错误: 用户名或密码错误', '/api/auth/login')

    await expect(call()).rejects.toThrow()

    expect(notifySessionEnded).not.toHaveBeenCalled()
  })
})

describe('其它状态码', () => {
  it('500 弹提示且不触发会话失效', async () => {
    const call = failWith(500, '服务器内部错误')

    await expect(call()).rejects.toThrow()

    expect(notifySessionEnded).not.toHaveBeenCalled()
    expect(showError).toHaveBeenCalledWith('服务器内部错误')
  })

  it('500 的错误不算 401（守卫仍按现状兜底）', async () => {
    const call = failWith(500, '服务器内部错误')

    await expect(call()).rejects.toSatisfy((e: unknown) => !isUnauthorized(e))
  })
})

describe('handleError 不再二次弹窗', () => {
  it('拦截器已展示过的错误，store 再走到也不弹第二条', () => {
    const call = failWith(500, '服务器内部错误')

    return call().catch((e) => {
      expect(showError).toHaveBeenCalledTimes(1)
      try {
        handleError(e)
      } catch {
        /* handleError 必定抛出，这里只是接住 */
      }
      expect(showError).toHaveBeenCalledTimes(1)
    })
  })

  it('从未展示过的错误仍然要弹（否则错误就静默消失了）', () => {
    try {
      handleError(new Error('本地校验没过'))
    } catch {
      /* 必定抛出 */
    }
    expect(showError).toHaveBeenCalledWith('本地校验没过')
  })
})
