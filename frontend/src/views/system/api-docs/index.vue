<template>
  <div class="api-docs-page" style="height: calc(100vh - 100px); width: 100%">
    <iframe
      :src="`${origin}/api/swagger-ui/index.html`"
      style="width: 100%; height: 100%; border: none; border-radius: 4px"
      title="API 接口文档"
    />
  </div>
</template>

<script setup lang="ts">
const origin = window.location.origin
</script>
