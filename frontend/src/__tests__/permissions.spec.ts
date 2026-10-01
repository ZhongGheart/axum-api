/**
 * 权限码 store 与 v-permission 指令
 *
 * 关键行为：按**权限码**判定（而非角色），且加载失败时 fail-closed。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import type { ObjectDirective } from 'vue'
import { usePermissionsStore } from '@/stores/permissions'
import { vPermission } from '@/directives/permission'
import { PERM } from '@/constants/permission'

const myPermissions = vi.fn()

vi.mock('@/api/auth', () => ({
  authApi: {
    myPermissions: () => myPermissions(),
  },
}))

vi.mock('@/api/helper', () => ({
  handleError: vi.fn(),
}))

/** 直接注入权限码，跳过网络 */
function withCodes(codes: string[]) {
  const store = usePermissionsStore()
  store.codes = new Set(codes)
  store.loaded = true
  return store
}

beforeEach(() => {
  setActivePinia(createPinia())
  myPermissions.mockReset()
})

describe('usePermissionsStore', () => {
  it('has 按权限码判定，与角色无关', () => {
    const store = withCodes([PERM.USER_CREATE])
    expect(store.has(PERM.USER_CREATE)).toBe(true)
    expect(store.has(PERM.USER_DELETE)).toBe(false)
  })

  it('hasAny 命中任一即可，hasAll 要求全部命中', () => {
    const store = withCodes([PERM.USER_UPDATE])
    expect(store.hasAny([PERM.USER_CREATE, PERM.USER_UPDATE])).toBe(true)
    expect(store.hasAll([PERM.USER_CREATE, PERM.USER_UPDATE])).toBe(false)
    expect(store.hasAll([PERM.USER_UPDATE])).toBe(true)
  })

  it('load 成功后记录权限码', async () => {
    myPermissions.mockResolvedValue([PERM.USER_LIST, PERM.USER_CREATE])
    const store = usePermissionsStore()
    const codes = await store.load()
    expect([...codes].sort()).toEqual([PERM.USER_CREATE, PERM.USER_LIST].sort())
    expect(store.loaded).toBe(true)
  })

  it('load 失败时 fail-closed：清空权限码且不标记已加载', async () => {
    myPermissions.mockRejectedValue(new Error('network down'))
    const store = usePermissionsStore()
    // 先给一点权限，确认失败后确实被清空
    store.codes = new Set([PERM.USER_DELETE])
    const codes = await store.load()
    expect(codes).toEqual([])
    expect(store.has(PERM.USER_DELETE)).toBe(false)
    expect(store.loaded).toBe(false)
  })

  it('reset 清空权限码，避免换账号残留', () => {
    const store = withCodes([PERM.USER_DELETE])
    store.reset()
    expect(store.has(PERM.USER_DELETE)).toBe(false)
    expect(store.loaded).toBe(false)
  })

  it('非数组响应按空集合处理，不抛异常', async () => {
    myPermissions.mockResolvedValue(null)
    const store = usePermissionsStore()
    await store.load()
    expect(store.codes.size).toBe(0)
  })
})

describe('v-permission 指令', () => {
  // Directive 是联合类型，这里取对象式指令分支
  const directive = vPermission as ObjectDirective

  function bind(value: string | string[]) {
    const parent = document.createElement('div')
    const el = document.createElement('button')
    parent.appendChild(el)
    const binding = { value } as never
    return { el, parent, binding }
  }

  it('有权限码时保留元素', () => {
    withCodes([PERM.USER_CREATE])
    const { el, parent, binding } = bind(PERM.USER_CREATE)
    directive.mounted?.(el, binding, null as never, null as never)
    expect(parent.contains(el)).toBe(true)
  })

  it('无权限码时从 DOM 移除', () => {
    withCodes([PERM.USER_LIST])
    const { el, parent, binding } = bind(PERM.USER_CREATE)
    directive.mounted?.(el, binding, null as never, null as never)
    expect(parent.contains(el)).toBe(false)
  })

  it('多权限码命中任一即保留', () => {
    withCodes([PERM.USER_UPDATE])
    const { el, parent, binding } = bind([PERM.USER_CREATE, PERM.USER_UPDATE])
    directive.mounted?.(el, binding, null as never, null as never)
    expect(parent.contains(el)).toBe(true)
  })

  it('权限码未加载（空集合）时移除元素，而非放行', () => {
    usePermissionsStore()
    const { el, parent, binding } = bind(PERM.USER_LIST)
    directive.mounted?.(el, binding, null as never, null as never)
    expect(parent.contains(el)).toBe(false)
  })
})
