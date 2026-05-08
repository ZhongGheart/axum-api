<template>
  <n-button v-if="hasPermission" v-bind="$attrs">
    <slot />
  </n-button>
</template>

<script setup lang="ts">
/**
 * 权限按钮组件
 *
 * 包装 Naive UI n-button，根据角色自动显隐。
 * 无权限时 DOM 彻底移除（v-if），非 disabled。
 *
 * 使用方式：
 *   <PermissionButton permission="admin" type="primary">新建用户</PermissionButton>
 *   <PermissionButton :permission="['admin']" size="small" @click="fn">编辑</PermissionButton>
 */
import { computed } from 'vue'
import { NButton } from 'naive-ui'
import { useUserStore } from '@/stores/user'

const props = withDefaults(
  defineProps<{
    /** 需要的权限角色 */
    permission: string | string[]
  }>(),
  {},
)

const userStore = useUserStore()

const hasPermission = computed(() => {
  if (!userStore.userInfo) return false
  const userRoles = (userStore.userInfo as unknown as { roles?: string[] }).roles || [
    (userStore.userInfo as unknown as { role: string }).role,
  ]
  const required = Array.isArray(props.permission) ? props.permission : [props.permission]
  return required.some((r: string) => userRoles.includes(r))
})
</script>
