<template>
  <n-config-provider :locale="zhCN" :date-locale="dateZhCN">
    <n-dialog-provider>
      <n-message-provider>
        <n-notification-provider>
          <router-view />
        </n-notification-provider>
      </n-message-provider>
    </n-dialog-provider>
  </n-config-provider>
</template>

<script setup lang="ts">
/**
 * 根组件
 *
 * 包装 Naive UI 全局 Provider：国际化、对话框、消息提示、通知。
 */
import { zhCN, dateZhCN } from 'naive-ui'
</script>
