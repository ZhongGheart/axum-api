<template>
  <div class="page-loading">
    <n-spin :show="true" :size="size">
      <div class="loading-content">
        <slot />
      </div>
    </n-spin>
  </div>
</template>

<script setup lang="ts">
/**
 * 页面加载动画组件
 *
 * 包裹内容区域，当 loading 为 true 时显示 spin 动画。
 * 通常配合 useAppStore 的全局 loading 状态使用。
 */
withDefaults(
  defineProps<{
    size?: 'small' | 'medium' | 'large'
  }>(),
  { size: 'large' },
)
</script>

<style scoped>
.page-loading {
  width: 100%;
  min-height: 200px;
}
.loading-content {
  width: 100%;
}
</style>
