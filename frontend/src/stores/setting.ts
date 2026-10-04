/**
 * 系统参数状态
 *
 * 只做一件事：**把服务端当前生效的口令策略取到前端**。
 * 管理页的读写走 `api/setting` 直接调接口，不进这个 store——
 * 参数列表是一次性的管理界面数据，塞进全局状态只会带来失效问题。
 *
 * ## 为什么口令策略必须来自服务端而不是前端常量
 *
 * 管理员能在「系统参数」页改最小长度、字符类别数、大小写混合。
 * 前端若仍按写死的 8 位 / 两类来提示，用户就会看到
 * 「界面说合规、后端说不合规」——这正是本仓反复修的那类缺陷
 * （界面在陈述一个它并不掌握的事实）。
 *
 * ## 降级策略
 *
 * 取不到时保留 [`DEFAULT_PASSWORD_POLICY`]，它与后端
 * `PasswordPolicy::default()` 逐字一致。**刻意不弹错误提示**：
 * 这是一个纯提示性增强，为它弹一条红字会让「服务端暂时不可达」
 * 看起来像「注册坏了」，而实际只是提示退回默认值、提交仍会照常校验。
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { settingApi } from '@/api/setting'
import type { PublicPasswordPolicy } from '@/api/setting'
import { DEFAULT_PASSWORD_POLICY } from '@/utils/password'
import type { PasswordPolicy } from '@/utils/password'

/** 端点返回的字段是可选的：服务端少给一个字段时不能变成 undefined 参与比较 */
function normalizePolicy(raw: PublicPasswordPolicy | null | undefined): PasswordPolicy {
  if (!raw) return DEFAULT_PASSWORD_POLICY
  const num = (v: unknown, fallback: number, min: number, max: number): number => {
    const n = typeof v === 'number' ? v : Number(v)
    if (!Number.isFinite(n)) return fallback
    return Math.min(max, Math.max(min, Math.trunc(n)))
  }
  return {
    min_length: num(raw.min_length, DEFAULT_PASSWORD_POLICY.min_length, 1, 1024),
    max_length: num(raw.max_length, DEFAULT_PASSWORD_POLICY.max_length, 1, 1024),
    min_char_classes: num(
      raw.min_char_classes,
      DEFAULT_PASSWORD_POLICY.min_char_classes,
      1,
      5,
    ),
    require_mixed_case: raw.require_mixed_case === true,
  }
}

export const useSettingStore = defineStore('setting', () => {
  const passwordPolicy = ref<PasswordPolicy>(DEFAULT_PASSWORD_POLICY)
  /** 是否已按当前会话加载过；避免登录页与注册页各发一次请求 */
  const policyLoaded = ref(false)
  const policyLoading = ref(false)

  /**
   * 拉取当前口令策略
   *
   * 幂等：已加载过就直接返回，除非显式 `force`。
   * 失败**不抛**——调用方都是页面挂载钩子，抛出去只会变成一条
   * 没有上下文的未捕获异常，而界面上退回默认值继续可用。
   */
  async function loadPasswordPolicy(force = false): Promise<PasswordPolicy> {
    if (policyLoaded.value && !force) return passwordPolicy.value
    if (policyLoading.value) return passwordPolicy.value
    policyLoading.value = true
    try {
      const raw = await settingApi.passwordPolicy()
      passwordPolicy.value = normalizePolicy(raw)
      policyLoaded.value = true
    } catch {
      // 保留回落策略；不提示（理由见文件头）
    } finally {
      policyLoading.value = false
    }
    return passwordPolicy.value
  }

  /** 登出或切换账号时清空，让下个会话重新拉一次 */
  function reset(): void {
    passwordPolicy.value = DEFAULT_PASSWORD_POLICY
    policyLoaded.value = false
  }

  return { passwordPolicy, policyLoaded, policyLoading, loadPasswordPolicy, reset }
})
