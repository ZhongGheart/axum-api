/**
 * 口令策略 store 的单元测试
 *
 * 这个 store 的职责只有一个：**把服务端策略取到前端，且取不到时不把界面带崩**。
 * 后者的重要性不低于前者——它是降级路径，出错时没人会在测试里发现，
 * 只会在某次参数表或网络出问题之后，用户看到一句莫名其妙的「注册失败」。
 */
/// <reference types="node" />
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { DEFAULT_PASSWORD_POLICY } from '@/utils/password'
import type { PublicPasswordPolicy } from '@/api/setting'

const passwordPolicy = vi.fn<() => Promise<PublicPasswordPolicy>>()

vi.mock('@/api/setting', () => ({
  settingApi: {
    // 用可变引用包一层，才能在测试之间替换实现
    passwordPolicy: (...args: unknown[]) => passwordPolicy(...(args as [])),
  },
}))

import { useSettingStore } from '@/stores/setting'

describe('useSettingStore.loadPasswordPolicy', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    passwordPolicy.mockReset()
  })

  it('取回服务端策略并替换回落值', async () => {
    passwordPolicy.mockResolvedValue({
      min_length: 12,
      max_length: 64,
      min_char_classes: 3,
      require_mixed_case: true,
    })
    const store = useSettingStore()
    await store.loadPasswordPolicy()
    expect(store.passwordPolicy).toEqual({
      min_length: 12,
      max_length: 64,
      min_char_classes: 3,
      require_mixed_case: true,
    })
    expect(store.policyLoaded).toBe(true)
  })

  /**
   * 失败时保留回落策略，**且不抛**
   *
   * 调用方都是页面挂载钩子。抛出去会变成一条没有上下文的未捕获异常，
   * 而界面上其实完全可用——提示退回默认值，提交仍会被后端照常校验。
   * 把它变成硬失败等于把一个纯提示性增强升级成了注册阻断。
   */
  it('接口失败时保留回落策略且不抛异常', async () => {
    passwordPolicy.mockRejectedValue(new Error('boom'))
    const store = useSettingStore()
    await expect(store.loadPasswordPolicy()).resolves.toBeDefined()
    expect(store.passwordPolicy).toEqual(DEFAULT_PASSWORD_POLICY)
    expect(store.policyLoaded).toBe(false)
  })

  it('已加载过时不重复请求（登录页与注册页会各调一次）', async () => {
    passwordPolicy.mockResolvedValue({
      min_length: 8,
      max_length: 128,
      min_char_classes: 2,
      require_mixed_case: false,
    })
    const store = useSettingStore()
    await store.loadPasswordPolicy()
    await store.loadPasswordPolicy()
    expect(passwordPolicy).toHaveBeenCalledTimes(1)
  })

  it('force = true 时重新拉取（参数被改过后需要刷新）', async () => {
    passwordPolicy.mockResolvedValue({
      min_length: 8,
      max_length: 128,
      min_char_classes: 2,
      require_mixed_case: false,
    })
    const store = useSettingStore()
    await store.loadPasswordPolicy()
    passwordPolicy.mockResolvedValue({
      min_length: 20,
      max_length: 128,
      min_char_classes: 2,
      require_mixed_case: false,
    })
    await store.loadPasswordPolicy(true)
    expect(passwordPolicy).toHaveBeenCalledTimes(2)
    expect(store.passwordPolicy.min_length).toBe(20)
  })

  /**
   * 归一化必须防住「服务端少给一个字段」
   *
   * 直接把响应赋给策略时，缺字段会让 `min_length` 变成 undefined，
   * 于是 `len < undefined` 恒为 false——**长度下界这条规则静默消失**，
   * 界面照常显示「至少 8 位」而实际不校验。这类缺陷不会有任何报错。
   */
  it('响应缺字段时用回落值补齐，不产生 undefined', async () => {
    passwordPolicy.mockResolvedValue({
      min_length: 12,
    } as unknown as PublicPasswordPolicy)
    const store = useSettingStore()
    await store.loadPasswordPolicy()
    expect(store.passwordPolicy.min_length).toBe(12)
    expect(store.passwordPolicy.max_length).toBe(DEFAULT_PASSWORD_POLICY.max_length)
    expect(store.passwordPolicy.min_char_classes).toBe(DEFAULT_PASSWORD_POLICY.min_char_classes)
    // 布尔缺省必须是 false 而不是 undefined：undefined 在条件里为假，
    // 而 `require_mixed_case: undefined` 会让「必须混合大小写」这条规则被跳过
    expect(store.passwordPolicy.require_mixed_case).toBe(false)
  })

  it('越界的数值被夹回合法区间（畸形响应不能让长度比较失去意义）', async () => {
    passwordPolicy.mockResolvedValue({
      min_length: -5,
      max_length: 99999,
      min_char_classes: 42,
      require_mixed_case: false,
    } as unknown as PublicPasswordPolicy)
    const store = useSettingStore()
    await store.loadPasswordPolicy()
    expect(store.passwordPolicy.min_length).toBe(1)
    expect(store.passwordPolicy.max_length).toBe(1024)
    expect(store.passwordPolicy.min_char_classes).toBe(5)
  })

  it('非布尔真值不算开启大小写混合（避免 "true" 字符串被当真）', async () => {
    passwordPolicy.mockResolvedValue({
      min_length: 8,
      max_length: 128,
      min_char_classes: 2,
      require_mixed_case: 'true' as unknown as boolean,
    })
    const store = useSettingStore()
    await store.loadPasswordPolicy()
    expect(store.passwordPolicy.require_mixed_case).toBe(false)
  })

  it('reset 后回到回落策略并允许重新加载', async () => {
    passwordPolicy.mockResolvedValue({
      min_length: 30,
      max_length: 128,
      min_char_classes: 2,
      require_mixed_case: false,
    })
    const store = useSettingStore()
    await store.loadPasswordPolicy()
    store.reset()
    expect(store.passwordPolicy).toEqual(DEFAULT_PASSWORD_POLICY)
    expect(store.policyLoaded).toBe(false)
  })
})
