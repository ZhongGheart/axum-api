<template>
  <div class="api-docs-page" style="height: calc(100vh - 100px); width: 100%">
    <iframe
      :src="swaggerUrl"
      style="width: 100%; height: 100%; border: none; border-radius: 4px"
      title="API 接口文档"
    />
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'

const swaggerUrl = ref('')

onMounted(() => {
  const origin = window.location.origin
  // 使用 CDN 加载的 Swagger UI，指向后端的 OpenAPI JSON
  swaggerUrl.value = `https://petstore.swagger.io/?url=${encodeURIComponent(origin + '/api/openapi.json')}`
})
</script>
