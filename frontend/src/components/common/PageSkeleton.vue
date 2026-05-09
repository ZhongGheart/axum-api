<template>
  <div class="skeleton-page">
    <!-- 标题骨架 -->
    <div class="skeleton-header">
      <div class="skeleton-line skeleton-title" />
      <div class="skeleton-line skeleton-subtitle" />
    </div>

    <!-- 搜索栏骨架 -->
    <div class="skeleton-search">
      <div class="skeleton-box" style="width: 200px; height: 34px" />
      <div class="skeleton-box" style="width: 100px; height: 34px" />
      <div class="skeleton-box" style="width: 120px; height: 34px; margin-left: auto" />
    </div>

    <!-- 表格骨架 -->
    <div class="skeleton-table">
      <!-- 表头 -->
      <div class="skeleton-table-header">
        <div
          v-for="col in columnCount"
          :key="'h-' + col"
          class="skeleton-line"
          :style="{ width: (60 + Math.random() * 30) + '%' }"
        />
      </div>
      <!-- 行 -->
      <div
        v-for="row in rowCount"
        :key="'r-' + row"
        class="skeleton-table-row"
      >
        <div
          v-for="col in columnCount"
          :key="'c-' + row + '-' + col"
          class="skeleton-cell"
          :style="{ width: (40 + Math.random() * 50) + '%' }"
        />
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 通用骨架屏组件
 *
 * 模拟页面加载中的占位效果，减少白屏感。
 * 可配合路由懒加载的 loading 状态使用。
 *
 * 使用方式：
 *   <PageSkeleton :row-count="5" :column-count="6" />
 */
withDefaults(
  defineProps<{
    /** 模拟行数 */
    rowCount?: number
    /** 模拟列数 */
    columnCount?: number
  }>(),
  { rowCount: 6, columnCount: 6 },
)
</script>

<style scoped>
.skeleton-page {
  padding: 16px;
  animation: skeleton-fade 1.5s ease-in-out infinite;
}

.skeleton-header {
  margin-bottom: 24px;
}

.skeleton-title {
  width: 180px;
  height: 28px;
  margin-bottom: 8px;
  background: linear-gradient(90deg, #f0f0f0 25%, #e0e0e0 50%, #f0f0f0 75%);
  background-size: 200% 100%;
  animation: skeleton-shine 1.5s ease-in-out infinite;
  border-radius: 4px;
}

.skeleton-subtitle {
  width: 280px;
  height: 16px;
  background: linear-gradient(90deg, #f0f0f0 25%, #e0e0e0 50%, #f0f0f0 75%);
  background-size: 200% 100%;
  animation: skeleton-shine 1.5s ease-in-out infinite;
  border-radius: 4px;
}

.skeleton-search {
  display: flex;
  gap: 12px;
  margin-bottom: 16px;
  align-items: center;
}

.skeleton-box {
  background: linear-gradient(90deg, #f0f0f0 25%, #e0e0e0 50%, #f0f0f0 75%);
  background-size: 200% 100%;
  animation: skeleton-shine 1.5s ease-in-out infinite;
  border-radius: 4px;
}

.skeleton-table {
  border: 1px solid #f0f0f0;
  border-radius: 4px;
  overflow: hidden;
}

.skeleton-table-header,
.skeleton-table-row {
  display: flex;
  gap: 16px;
  padding: 12px 16px;
  border-bottom: 1px solid #f0f0f0;
}

.skeleton-table-header {
  background: #fafafa;
}

.skeleton-line {
  height: 14px;
  background: linear-gradient(90deg, #f0f0f0 25%, #e0e0e0 50%, #f0f0f0 75%);
  background-size: 200% 100%;
  animation: skeleton-shine 1.5s ease-in-out infinite;
  border-radius: 4px;
  flex: 1;
}

.skeleton-cell {
  flex: 1;
  height: 14px;
  background: linear-gradient(90deg, #f0f0f0 25%, #e0e0e0 50%, #f0f0f0 75%);
  background-size: 200% 100%;
  animation: skeleton-shine 1.5s ease-in-out infinite;
  border-radius: 4px;
}

.skeleton-table-row:last-child {
  border-bottom: none;
}

@keyframes skeleton-shine {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}

@keyframes skeleton-fade {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.85; }
}
</style>
