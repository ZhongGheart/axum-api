/**
 * 两步验证 API 契约（v0.25.0）
 *
 * 钉住**请求体形状**：字段名写错时后端 `deny_unknown_fields` 会直接 400，
 * 而如果前后端字段名恰好都写错成同一个词，错误会一路静默到"登录不了"。
 * 这类契约值得用测试钉住，和 `roleApi.spec.ts` 钉 `menu_ids` 是同一个理由。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'

const { get, post } = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
}))

vi.mock('@/api/index', () => ({
  default: { get, post },
}))

import { authApi } from '@/api/auth'

beforeEach(() => {
  get.mockReset()
  post.mockReset()
})

describe('两步验证端点', () => {
  it('状态走 GET /auth/2fa', () => {
    authApi.twoFactorStatus()
    expect(get).toHaveBeenCalledWith('/auth/2fa')
  })

  it('setup/enable/disable 挂在 auth 下（当前用户自助）', () => {
    authApi.setupTwoFactor()
    authApi.enableTwoFactor('123456')
    authApi.disableTwoFactor('secret')
    expect(post.mock.calls.map((c) => c[0])).toEqual([
      '/auth/2fa/setup',
      '/auth/2fa/enable',
      '/auth/2fa/disable',
    ])
  })

  it('enable 带 code', () => {
    authApi.enableTwoFactor('934196')
    expect(post).toHaveBeenCalledWith('/auth/2fa/enable', { code: '934196' })
  })

  it('disable 带 password（关闭 2FA 必须出示当前口令）', () => {
    authApi.disableTwoFactor('Str0ng-Passw0rd!')
    expect(post).toHaveBeenCalledWith('/auth/2fa/disable', { password: 'Str0ng-Passw0rd!' })
  })

  it('重新生成恢复码是无请求体的 POST', () => {
    authApi.regenerateRecoveryCodes()
    expect(post).toHaveBeenCalledWith('/auth/2fa/recovery-codes')
  })

  /**
   * verify 是唯一的公开端点：字段名必须是 challenge_token 与 code。
   * 写成 challengeToken（驼峰）会被后端 deny_unknown_fields 拒掉，
   * 症状是"第二次验证永远失败"，很难自己联想到是字段名。
   */
  it('verify 带 challenge_token 与 code，且走公开路径', () => {
    authApi.verifyTwoFactor('ch-abc', '654321')
    expect(post).toHaveBeenCalledWith('/auth/2fa/verify', {
      challenge_token: 'ch-abc',
      code: '654321',
    })
  })
})
