<template>
  <n-button v-if="hasPermission" v-bind="$attrs">
    <slot />
  </n-button>
</template>

<script setup lang="ts">
/**
 * 权限按钮组件
 *
 * 包装 Naive UI n-button，根据**权限码**自动显隐。
 * 无权限时 DOM 彻底移除（v-if），非 disabled。
 *
 * 使用方式：
 *   <PermissionButton permission="system:user:create" type="primary">新建用户</PermissionButton>
 *   <PermissionButton permission="system:user:update" size="small" @click="fn">编辑</PermissionButton>
 */
import { computed } from 'vue'
import { NButton } from 'naive-ui'
import { usePermissionsStore } from '@/stores/permissions'

const props = withDefaults(
  defineProps<{
    /** 需要的权限码（任一命中即显示） */
    permission: string | string[]
  }>(),
  {},
)

const permissionsStore = usePermissionsStore()

const hasPermission = computed(() => permissionsStore.hasAny(props.permission))
</script>
