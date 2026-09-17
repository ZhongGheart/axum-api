/**
 * 导航菜单状态
 *
 * 后端菜单是导航的唯一来源：登录后拉取一次，供侧栏渲染与动态路由注册使用。
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { menuApi, type MenuNode } from '@/api/menu'
import { handleError } from '@/api/helper'

export const useMenuStore = defineStore('menu', () => {
  /** 当前用户可见的菜单树 */
  const menus = ref<MenuNode[]>([])
  /** 是否已按当前会话加载完成 */
  const loaded = ref(false)
  /** 加载中标记，避免并发重复请求 */
  const loading = ref(false)

  async function load(): Promise<MenuNode[]> {
    if (loading.value) return menus.value
    loading.value = true
    try {
      const data = (await menuApi.myMenus()) as unknown as MenuNode[]
      menus.value = Array.isArray(data) ? data : []
      loaded.value = true
      return menus.value
    } catch (error) {
      loaded.value = false
      handleError(error)
      return []
    } finally {
      loading.value = false
    }
  }

  /** 登出或切换账号时清空 */
  function reset(): void {
    menus.value = []
    loaded.value = false
    loading.value = false
  }

  return { menus, loaded, loading, load, reset }
})
