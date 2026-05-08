/**
 * 应用级全局状态
 *
 * 管理主题、侧栏折叠、加载状态等公共配置。
 */
import { defineStore } from 'pinia'
import { ref, watch } from 'vue'
import { useOsTheme } from 'naive-ui'
import type { GlobalThemeOverrides } from 'naive-ui'

/** 主题类型 */
export type ThemeType = 'light' | 'dark'

export const useAppStore = defineStore('app', () => {
  /** 操作系统主题 */
  const osTheme = useOsTheme()

  /** 当前主题（默认跟随系统） */
  const theme = ref<ThemeType>(osTheme.value || 'light')

  /** 是否暗黑模式 */
  const isDark = ref(theme.value === 'dark')

  /** 侧栏是否折叠 */
  const collapsed = ref(false)

  /** 全局加载中 */
  const loading = ref(false)

  /** 加载提示文字 */
  const loadingText = ref('加载中...')

  // 监听主题变化同步 isDark 和 data-theme 属性
  watch(theme, (val) => {
    isDark.value = val === 'dark'
    document.documentElement.setAttribute('data-theme', val)
  })

  // 监听系统主题变化
  watch(osTheme, (val) => {
    if (val) theme.value = val
  })

  /** 切换主题 */
  function toggleTheme() {
    theme.value = theme.value === 'light' ? 'dark' : 'light'
  }

  /** 设置主题 */
  function setTheme(t: ThemeType) {
    theme.value = t
  }

  /** 切换侧栏 */
  function toggleCollapsed() {
    collapsed.value = !collapsed.value
  }

  /** 设置全局加载状态 */
  function setLoading(val: boolean, text = '加载中...') {
    loading.value = val
    if (val) loadingText.value = text
  }

  /** 亮色主题覆盖 */
  const lightThemeOverrides: GlobalThemeOverrides = {
    common: {
      primaryColor: '#2080f0',
      primaryColorHover: '#4098fc',
      bodyColor: '#f5f7fa',
      cardColor: '#ffffff',
      modalColor: '#ffffff',
      popoverColor: '#ffffff',
      inputColor: '#ffffff',
    },
  }

  /** 深色主题覆盖 */
  const darkThemeOverrides: GlobalThemeOverrides = {
    common: {
      primaryColor: '#70c0e8',
      primaryColorHover: '#8cd4f5',
      bodyColor: '#101014',
      cardColor: '#1e1e22',
      modalColor: '#1e1e22',
      popoverColor: '#1e1e22',
      inputColor: '#2a2a30',
      placeholderColor: '#666',
      placeholderColorDisabled: '#444',
      textColor1: '#e5e5e5',
      textColor2: '#cccccc',
      textColor3: '#999999',
    },
  }

  return {
    theme,
    isDark,
    collapsed,
    loading,
    loadingText,
    toggleTheme,
    setTheme,
    toggleCollapsed,
    setLoading,
    lightThemeOverrides,
    darkThemeOverrides,
  }
})
