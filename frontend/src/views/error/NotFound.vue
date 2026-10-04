<template>
  <div class="not-found">
    <h1>404</h1>
    <p>页面未找到</p>
    <router-link to="/">返回首页</router-link>
  </div>
</template>

<script setup lang="ts">
/**
 * 404 错误页面
 */
</script>

<style scoped>
.not-found {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100vh;
  gap: 16px;
}

.not-found h1 {
  font-size: 72px;
  font-weight: 600;
  line-height: 1;
  color: var(--text-tertiary);
}

.not-found p {
  font-size: 16px;
  color: var(--text-secondary);
}

.not-found a {
  font-size: 14px;
  color: var(--primary-color);
}

.not-found a:hover {
  text-decoration: underline;
}
</style>
