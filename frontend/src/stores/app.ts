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

  // 监听主题变化同步 isDark 和 data-theme 属性（immediate 确保初始化时同步）
  watch(theme, (val) => {
    isDark.value = val === 'dark'
    document.documentElement.setAttribute('data-theme', val)
  }, { immediate: true })

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

  /**
   * 设计令牌（亮色）
   *
   * 单一事实源：CSS 变量表（`assets/styles/global.css`）与这里的 `common`
   * 用的是同一套色值。改主色时两处必须一起改，否则会出现
   * "按钮是新的、链接还是旧的"这种半迁移状态。
   *
   * 主色刻意避开纯蓝和紫蓝：后台系统用蓝紫最容易滑向"通用模板感"，
   * 这套选的是偏青的靛蓝（#2b5fd9 → hover #3f74f0），
   * 在中性灰底上足够跳，又不会和状态色（绿/琥珀/红）撞色。
   */
  const LIGHT_TOKENS = {
    primaryColor: '#2b5fd9',
    primaryColorHover: '#3f74f0',
    primaryColorPressed: '#244bb5',
    primaryColorSuppl: '#2b5fd9',
    successColor: '#1f9254',
    successColorHover: '#27a962',
    warningColor: '#b8730c',
    errorColor: '#c8373d',
    infoColor: '#2b5fd9',
  } as const

  const DARK_TOKENS = {
    primaryColor: '#5b8cf5',
    primaryColorHover: '#7aa5f8',
    primaryColorPressed: '#4a79e0',
    primaryColorSuppl: '#5b8cf5',
    successColor: '#3fb877',
    successColorHover: '#57c78d',
    warningColor: '#e0a23c',
    errorColor: '#e2666c',
    infoColor: '#5b8cf5',
  } as const

  /** 亮色主题覆盖 */
  const lightThemeOverrides: GlobalThemeOverrides = {
    common: {
      ...LIGHT_TOKENS,
      // 表面：中性灰阶（原 #f5f7fa 偏蓝，和主色一起看整屏发灰泛蓝）
      bodyColor: '#f4f5f7',
      cardColor: '#ffffff',
      modalColor: '#ffffff',
      popoverColor: '#ffffff',
      tableColor: '#ffffff',
      inputColor: '#ffffff',
      hoverColor: '#f2f3f5',
      borderColor: '#e3e5e9',
      dividerColor: '#eceef1',
      textColorBase: '#1c1f26',
      textColor1: '#1c1f26',
      textColor2: '#5a6070',
      textColor3: '#8b909e',
      placeholderColor: '#9aa0ad',
      placeholderColorDisabled: '#b8bdc7',
      borderRadius: '6px',
      fontWeightStrong: '600',
    },
    Card: {
      borderRadius: '8px',
      borderColor: '#e6e8ec',
      paddingMedium: '18px 20px',
      paddingLarge: '22px 24px',
      titleFontWeight: '600',
      boxShadow: '0 1px 2px rgba(20, 24, 34, 0.04)',
    },
    DataTable: {
      thColor: '#fafbfc',
      thColorHover: '#f4f5f7',
      thTextColor: '#5a6070',
      thFontWeight: '600',
      borderColor: '#e6e8ec',
      tdColorHover: '#f7f9fc',
      borderRadius: '8px',
      fontSizeSmall: '13px',
      fontSizeMedium: '13px',
    },
    Button: {
      borderRadiusSmall: '5px',
      borderRadiusMedium: '6px',
      borderRadiusLarge: '8px',
      fontWeight: '400',
      fontWeightStrong: '500',
      // 默认描边按钮在灰底上边框偏淡，调深一档才有可点击的边界感
      border: '1px solid #d5d9e0',
      borderHover: '1px solid #b6bcc7',
      borderPressed: '1px solid #9aa2b0',
      textColor: '#3d4351',
      textColorHover: '#1c1f26',
    },
    Menu: {
      itemHeight: '42px',
      borderRadius: '6px',
      itemColorHover: '#f0f2f6',
      itemColorActive: '#e8eefc',
      itemColorActiveHover: '#dfe6f9',
      itemTextColorActive: '#2b5fd9',
      itemTextColorChildActive: '#2b5fd9',
      itemTextColorChildActiveHover: '#2b5fd9',
      itemIconSize: '18px',
      arrowColor: '#9aa0ad',
      arrowColorHover: '#5a6070',
      fontSize: '14px',
    },
    Input: {
      borderRadius: '6px',
      border: '1px solid #d5d9e0',
      borderHover: '1px solid #b6bcc7',
      borderFocus: '1px solid #2b5fd9',
      placeholderColor: '#9aa0ad',
      color: '#ffffff',
    },
    InternalSelection: {
      borderRadius: '6px',
      border: '1px solid #d5d9e0',
      borderHover: '1px solid #b6bcc7',
      borderFocus: '1px solid #2b5fd9',
      borderActive: '1px solid #2b5fd9',
      placeholderColor: '#9aa0ad',
    },
    Layout: {
      siderColor: '#ffffff',
      siderBorderColor: '#e6e8ec',
      headerColor: '#ffffff',
      headerBorderColor: '#e6e8ec',
    },
  }

  /** 深色主题覆盖 */
  const darkThemeOverrides: GlobalThemeOverrides = {
    common: {
      ...DARK_TOKENS,
      bodyColor: '#14161a',
      cardColor: '#1c1f25',
      modalColor: '#232730',
      popoverColor: '#232730',
      tableColor: '#1c1f25',
      inputColor: '#22262e',
      hoverColor: '#2a2f39',
      borderColor: '#2e333d',
      dividerColor: '#282d36',
      textColorBase: '#e8eaee',
      textColor1: '#e8eaee',
      textColor2: '#a8aeb9',
      textColor3: '#7e8593',
      // 原 #666 在 #22262e 上对比度约 3.4:1，够不到正文下限
      placeholderColor: '#848b98',
      placeholderColorDisabled: '#5f6673',
      borderRadius: '6px',
      fontWeightStrong: '600',
    },
    Card: {
      borderRadius: '8px',
      borderColor: '#2e333d',
      paddingMedium: '18px 20px',
      paddingLarge: '22px 24px',
      titleFontWeight: '600',
      boxShadow: '0 1px 2px rgba(0, 0, 0, 0.32)',
    },
    DataTable: {
      thColor: '#22262e',
      thColorHover: '#2a2f39',
      thTextColor: '#a8aeb9',
      thFontWeight: '600',
      borderColor: '#2e333d',
      tdColorHover: '#232830',
      borderRadius: '8px',
      fontSizeSmall: '13px',
      fontSizeMedium: '13px',
    },
    Button: {
      borderRadiusSmall: '5px',
      borderRadiusMedium: '6px',
      borderRadiusLarge: '8px',
      fontWeight: '400',
      fontWeightStrong: '500',
      border: '1px solid #3a414d',
      borderHover: '1px solid #4c5462',
      borderPressed: '1px solid #5f6878',
      textColor: '#c3c8d1',
      textColorHover: '#f0f2f5',
    },
    Menu: {
      itemHeight: '42px',
      borderRadius: '6px',
      itemColorHover: '#252a33',
      itemColorActive: '#24304f',
      itemColorActiveHover: '#2a3859',
      itemTextColorActive: '#8fb0ff',
      itemTextColorChildActive: '#8fb0ff',
      itemTextColorChildActiveHover: '#8fb0ff',
      itemIconSize: '18px',
      arrowColor: '#7e8593',
      arrowColorHover: '#a8aeb9',
      fontSize: '14px',
    },
    Input: {
      borderRadius: '6px',
      border: '1px solid #3a414d',
      borderHover: '1px solid #4c5462',
      borderFocus: '1px solid #5b8cf5',
      placeholderColor: '#848b98',
      color: '#22262e',
    },
    InternalSelection: {
      borderRadius: '6px',
      border: '1px solid #3a414d',
      borderHover: '1px solid #4c5462',
      borderFocus: '1px solid #5b8cf5',
      borderActive: '1px solid #5b8cf5',
      placeholderColor: '#848b98',
    },
    Layout: {
      siderColor: '#1c1f25',
      siderBorderColor: '#2e333d',
      headerColor: '#1c1f25',
      headerBorderColor: '#2e333d',
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
