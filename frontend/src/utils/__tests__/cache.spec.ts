import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { requestCache } from '@/utils/cache'

describe('requestCache', () => {
  beforeEach(() => {
    requestCache.invalidate()
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('caches GET responses and returns them before TTL expires', () => {
    requestCache.set('GET', '/api/admin/users', [{ id: 1 }], { page: 1 })

    expect(requestCache.get('GET', '/api/admin/users', { page: 1 })).toEqual([{ id: 1 }])
  })

  it('separates cache entries by query params', () => {
    requestCache.set('GET', '/api/admin/users', ['page-1'], { page: 1 })

    expect(requestCache.get('GET', '/api/admin/users', { page: 2 })).toBeNull()
  })

  it('drops entries after their TTL', () => {
    requestCache.set('GET', '/api/admin/users', ['cached'], { page: 1 })

    vi.advanceTimersByTime(30_001)

    expect(requestCache.get('GET', '/api/admin/users', { page: 1 })).toBeNull()
  })

  it('invalidates every entry (used on login, logout and after writes)', () => {
    requestCache.set('GET', '/api/auth/me', { username: 'alice' })
    requestCache.set('GET', '/api/admin/users', [])

    requestCache.invalidate()

    expect(requestCache.get('GET', '/api/auth/me')).toBeNull()
    expect(requestCache.get('GET', '/api/admin/users')).toBeNull()
  })

  it('never caches non-GET responses', () => {
    requestCache.set('POST', '/api/admin/users', { id: 'created' })

    expect(requestCache.get('POST', '/api/admin/users')).toBeNull()
  })
})
