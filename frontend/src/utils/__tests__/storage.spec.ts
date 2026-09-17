import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  clearStorage,
  getStorage,
  getToken,
  removeToken,
  setStorage,
  setToken,
} from '@/utils/storage'

describe('storage', () => {
  beforeEach(() => {
    clearStorage()
    vi.useRealTimers()
  })

  it('round-trips values through localStorage', () => {
    setStorage('demo', { name: 'axum', roles: ['admin'] })

    expect(getStorage('demo')).toEqual({ name: 'axum', roles: ['admin'] })
  })

  it('obfuscates stored values (base64 混淆，非加密)', () => {
    setToken('secret-token-value')

    // 直接读原始 localStorage 不应出现明文
    const raw = window.localStorage.getItem('axum_token') ?? ''
    expect(raw).not.toContain('secret-token-value')
    // 但通过封装仍能读回
    expect(getToken()).toBe('secret-token-value')
  })

  it('expires entries after the configured TTL', () => {
    vi.useFakeTimers()
    setStorage('temp', 'value', 1000)

    vi.advanceTimersByTime(1001)

    expect(getStorage('temp')).toBeUndefined()
  })

  it('removes individual entries and clears prefixed entries', () => {
    setToken('token-a')
    setStorage('other', 'value')

    removeToken()
    expect(getToken()).toBeUndefined()

    clearStorage()
    expect(getStorage('other')).toBeUndefined()
  })
})
