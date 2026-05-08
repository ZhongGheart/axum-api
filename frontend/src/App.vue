<template>
  <n-config-provider
    :locale="zhCN"
    :date-locale="dateZhCN"
    :theme="appStore.isDark ? darkTheme : null"
    :theme-overrides="appStore.isDark ? appStore.darkThemeOverrides : appStore.lightThemeOverrides"
  >
    <n-loading-bar-provider>
      <n-dialog-provider>
        <n-message-provider>
          <n-notification-provider>
            <!-- 全局加载遮罩 -->
            <n-spin v-if="appStore.loading" :show="true" content-class="global-loading">
              <router-view />
            </n-spin>
            <router-view v-else />
          </n-notification-provider>
        </n-message-provider>
      </n-dialog-provider>
    </n-loading-bar-provider>
  </n-config-provider>
</template>

<script setup lang="ts">
/**
 * 根组件
 *
 * 包装 Naive UI 全局 Provider：国际化、对话框、消息提示、通知、加载条。
 * 支持亮色/暗黑主题动态切换。
 */
import { zhCN, dateZhCN, darkTheme } from 'naive-ui'
import { useMessage, useDialog, useNotification, useLoadingBar } from 'naive-ui'
import { registerGlobalApis } from '@/utils/message'
import { useAppStore } from '@/stores/app'

const appStore = useAppStore()

// 注入全局 loading bar
const loadingBar = useLoadingBar()
// 注入全局消息/弹窗/通知 API
registerGlobalApis(useMessage(), useDialog(), useNotification())

// 暴露 loadingBar 给全局使用
window.$loadingBar = loadingBar
</script>

<style>
/* 全局加载遮罩 */
.global-loading {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
}
</style>
