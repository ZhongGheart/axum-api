<template>
  <!-- 此组件无 UI，仅在 setup 中注册全局 API -->
</template>

<script setup lang="ts">
/**
 * 全局 API 注册组件
 *
 * 在 n-loading-bar-provider / n-dialog-provider / n-message-provider /
 * n-notification-provider 的子孙节点中调用 useXxx()，确保 provider 已存在。
 */
import { useMessage, useDialog, useNotification, useLoadingBar } from 'naive-ui'
import { registerGlobalApis } from '@/utils/message'

// 注入全局 loading bar（需要在 n-loading-bar-provider 内调用）
const loadingBar = useLoadingBar()
window.$loadingBar = loadingBar

// 注入全局消息/弹窗/通知 API
registerGlobalApis(useMessage(), useDialog(), useNotification())
</script>
