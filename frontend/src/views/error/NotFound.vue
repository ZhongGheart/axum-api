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
  color: #d03050;
}

.not-found p {
  font-size: 18px;
  color: #666;
}

.not-found a {
  color: #2080f0;
  text-decoration: underline;
}
</style>
