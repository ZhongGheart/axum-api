<template>
  <div class="base-table">
    <n-data-table
      :columns="columns"
      :data="data"
      :loading="loading"
      :bordered="bordered"
      :size="size"
      :single-line="singleLine"
      :row-key="rowKey"
      :striped="striped"
      :max-height="maxHeight"
      :flex-height="flexHeight"
      :virtual-scroll="virtualScroll"
      class="base-table__table"
    />

    <!-- 分页 -->
    <div v-if="showPagination" class="base-table__pagination">
      <n-pagination
        v-model:page="currentPage"
        :page-count="pageCount"
        :page-size="pageSize"
        :page-sizes="pageSizes"
        :show-size-picker="showSizePicker"
        :show-quick-jumper="showQuickJumper"
        @update:page="onPageChange"
        @update:page-size="onPageSizeChange"
      />
      <span class="base-table__total">共 {{ total }} 条</span>
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 通用数据表格组件
 *
 * 封装 Naive UI n-data-table + n-pagination，统一处理分页逻辑。
 *
 * 使用方式：
 *   <BaseTable
 *     :columns="columns"
 *     :data="data"
 *     :loading="loading"
 *     :total="total"
 *     :page="page"
 *     @update:page="onPageChange"
 *     @update:page-size="onPageSizeChange"
 *   />
 */
import { computed } from 'vue'
import type { DataTableColumn, DataTableSize } from 'naive-ui'

const props = withDefaults(
  defineProps<{
    columns: DataTableColumn[]
    data: Record<string, unknown>[]
    loading?: boolean
    total?: number
    page?: number
    pageSize?: number
    pageSizes?: number[]
    pageCount?: number
    bordered?: boolean
    size?: DataTableSize
    singleLine?: boolean
    striped?: boolean
    maxHeight?: string | number
    flexHeight?: boolean
    virtualScroll?: boolean
    rowKey?: string | ((row: Record<string, unknown>) => string | number)
    showPagination?: boolean
    showSizePicker?: boolean
    showQuickJumper?: boolean
  }>(),
  {
    loading: false,
    total: 0,
    page: 1,
    pageSize: 10,
    pageSizes: () => [10, 20, 50],
    bordered: true,
    size: 'small',
    singleLine: true,
    striped: false,
    showPagination: true,
    showSizePicker: true,
    showQuickJumper: false,
  },
)

const emit = defineEmits<{
  'update:page': [page: number]
  'update:page-size': [size: number]
}>()

const currentPage = computed({
  get: () => props.page,
  set: (val: number) => emit('update:page', val),
})

const pageCount = computed(() => {
  if (props.pageCount) return props.pageCount
  if (props.total > 0 && props.pageSize > 0) {
    return Math.ceil(props.total / props.pageSize)
  }
  return 1
})

function onPageChange(page: number) {
  emit('update:page', page)
}

function onPageSizeChange(size: number) {
  emit('update:page-size', size)
}
</script>

<style scoped>
.base-table {
  width: 100%;
}

.base-table__pagination {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding: 12px 0;
  gap: 12px;
}

.base-table__total {
  font-size: 13px;
  color: #888;
}
</style>
