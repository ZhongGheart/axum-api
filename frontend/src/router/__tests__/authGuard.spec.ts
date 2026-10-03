/**
 * 路由守卫在 401 下的行为
 *
 * **为什么 e2e 抓不住这个分支**：401 发生时，响应拦截器注册的 handler 已经先
 * 一步 `router.replace('/login')`，vue-router 随即以 `pendingLocation` 变了为由
 * 取消原导航。于是守卫无论 `next(false)` 还是 `next()`，浏览器最后都停在登录页，
 * e2e 全绿——注入实验（把它改回 `next()`）实测 27/27 依旧通过。
 *
 * 结论是诚实的：这一行在当前接线方式下属于**纵深防御**，不是承重路径。
 * 但它仍必须被钉住——handler 的注册时机一旦变化（例如某个入口忘了 import
 * `router`，`notifySessionEnded` 就静默无操作），此刻没人拦住的 404 兜底会回来。
 * 单元测试能直接观察守卫传了哪个 `next`，这是 e2e 做不到的。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NavigationFailureType } from 'vue-router'
import { ApiError } from '@/api/errors'

vi.mock('@/utils/storage', () => ({ getToken: vi.fn(() => validToken()) }))
vi.mock('@/utils/session', () => ({
  notifySessionEnded: vi.fn(),
  registerSessionEndedHandler: vi.fn(),
}))

const menuLoad = vi.fn()
const permissionsLoad = vi.fn()

vi.mock('@/stores/menu', () => ({
  useMenuStore: () => ({ loaded: false, load: menuLoad }),
}))
vi.mock('@/stores/permissions', () => ({
  usePermissionsStore: () => ({ loaded: true, load: permissionsLoad }),
}))
vi.mock('@/stores/user', () => ({
  useUserStore: () => ({ mustChangePassword: false, clearLocalSession: vi.fn() }),
}))

/** 一枚**未过期**的 JWT：守卫的过期检查必须放行，否则测的就不是 401 分支 */
function validToken(): string {
  const payload = btoa(JSON.stringify({ exp: Math.floor(Date.now() / 1000) + 3600 }))
  return `h.${payload}.s`
}

const { default: router } = await import('@/router/index')

beforeEach(() => {
  menuLoad.mockReset()
  permissionsLoad.mockReset()
})

describe('守卫：菜单加载遇到 401', () => {
  it('中止导航，绝不落到 404 兜底（会话没了不等于页面不存在）', async () => {
    menuLoad.mockRejectedValue(new ApiError('令牌已被注销，请重新登录', 401, true))

    const failure = await router.push('/system/user').catch(() => undefined)

    // next(false) → vue-router 以 aborted 结束本次导航
    expect(failure?.type).toBe(NavigationFailureType.aborted)
    expect(router.currentRoute.value.path).not.toBe('/system/user')
  })

  it('换 401 以外的失败则维持放行（网络不通不该把用户锁死在原页）', async () => {
    menuLoad.mockRejectedValue(new ApiError('网络异常', 0, true))

    // 目标必须与上一条不同：vue-router 对**同一目标**的第二次 push 直接判定
    // duplicated 并跳过守卫，那这条断言就会因为"根本没跑守卫"而空过。
    await router.push('/system/role').catch(() => undefined)

    // next() → 导航成立；业务路由没注册，于是落在 404 上
    expect(router.currentRoute.value.path).toBe('/system/role')
  })
})

describe('守卫：令牌已过期', () => {
  it('中止导航并走会话失效出口（而不是 localStorage.clear()）', async () => {
    const { getToken } = await import('@/utils/storage')
    const { notifySessionEnded } = await import('@/utils/session')
    vi.mocked(getToken).mockReturnValueOnce('h.' + btoa(JSON.stringify({ exp: 1 })) + '.s')

    const failure = await router.push('/system/menu').catch(() => undefined)

    expect(notifySessionEnded).toHaveBeenCalledWith('登录状态已过期，请重新登录')
    expect(failure?.type).toBe(NavigationFailureType.aborted)
  })
})
