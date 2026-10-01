/**
 * 权限码状态
 *
 * 权限码来自 `GET /api/auth/permissions`，与后端 `PermissionGuard`
 * 使用同一份数据源（`menus.permission` 的 `type='button'` 行）。
 *
 * 关键区别：此前 `v-permission` / `PermissionButton` 按**角色**判定，
 * 只能表达「admin 可见、user 不可见」；改为按**权限码**判定后，
 * 按钮显隐与后端接口的放行条件一致——前端藏起来的按钮，后端一定也拒绝。
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { authApi } from '@/api/auth'
import { handleError } from '@/api/helper'

export const usePermissionsStore = defineStore('permissions', () => {
  /** 当前用户的权限码集合 */
  const codes = ref<Set<string>>(new Set())
  /** 是否已按当前会话加载完成 */
  const loaded = ref(false)
  /** 加载中标记，避免并发重复请求 */
  const loading = ref(false)

  async function load(): Promise<string[]> {
    if (loading.value) return [...codes.value]
    loading.value = true
    try {
      const data = (await authApi.myPermissions()) as unknown as string[]
      codes.value = new Set(Array.isArray(data) ? data : [])
      loaded.value = true
      return [...codes.value]
    } catch (error) {
      // 加载失败时保持空集合：按钮全部隐藏是 fail-closed，
      // 宁可少显示也不能显示一个点下去必然 403 的按钮
      codes.value = new Set()
      loaded.value = false
      handleError(error)
      return []
    } finally {
      loading.value = false
    }
  }

  /** 是否拥有指定权限码 */
  function has(code: string): boolean {
    return codes.value.has(code)
  }

  /** 是否拥有其中任一权限码 */
  function hasAny(required: string | string[]): boolean {
    const list = Array.isArray(required) ? required : [required]
    return list.some((code) => codes.value.has(code))
  }

  /** 是否拥有全部权限码 */
  function hasAll(required: string[]): boolean {
    return required.every((code) => codes.value.has(code))
  }

  /** 登出或切换账号时清空，避免上一账号的权限码残留 */
  function reset(): void {
    codes.value = new Set()
    loaded.value = false
    loading.value = false
  }

  return { codes, loaded, loading, load, has, hasAny, hasAll, reset }
})
