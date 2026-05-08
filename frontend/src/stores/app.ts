/**
 * 应用级全局状态
 *
 * 管理主题、侧栏折叠、加载状态等公共配置。
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useAppStore = defineStore('app', () => {
  /** 侧栏是否折叠 */
  const collapsed = ref(false)

  /** 全局加载中 */
  const loading = ref(false)

  /** 切换侧栏 */
  function toggleCollapsed() {
    collapsed.value = !collapsed.value
  }

  return {
    collapsed,
    loading,
    toggleCollapsed,
  }
})
