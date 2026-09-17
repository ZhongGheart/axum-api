<template>
  <div class="enhanced-table">
    <!-- 工具栏：批量操作 + 搜索 + 列显隐 -->
    <div v-if="showToolbar" class="enhanced-table__toolbar">
      <n-space align="center" wrap>
        <slot name="toolbar-left" />

        <!-- 批量删除按钮 -->
        <n-button
          v-if="batchActions.delete && checkedRowKeys.length > 0"
          type="error"
          size="small"
          @click="$emit('batchDelete', checkedRowKeys)"
        >
          批量删除 ({{ checkedRowKeys.length }})
        </n-button>

        <n-space style="margin-left:auto" align="center">
          <n-input
            v-if="showSearch"
            v-model:value="searchKeyword"
            :placeholder="searchPlaceholder"
            clearable
            style="width:200px"
            @input="onSearch"
          />
          <slot name="toolbar-right" />
        </n-space>
      </n-space>
    </div>

    <!-- 表格 -->
    <n-data-table
      ref="tableRef"
      :columns="mergedColumns"
      :data="data"
      :loading="loading"
      :bordered="bordered"
      :size="size"
      :single-line="singleLine"
      :striped="striped"
      :max-height="maxHeight"
      :flex-height="flexHeight"
      :row-key="rowKey"
      :checked-row-keys="checkedRowKeys"
      @update:checked-row-keys="onCheckChange"
      @update:sorter="onSortChange"
      class="enhanced-table__table"
    />

    <!-- 分页 -->
    <div v-if="showPagination" class="enhanced-table__pagination">
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
      <span class="enhanced-table__total">共 {{ total }} 条</span>
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 增强表格组件
 *
 * 支持分页、排序、筛选、批量操作、行编辑。
 *
 * 使用方式：
 *   <EnhancedBaseTable
 *     :columns="columns"
 *     :data="data"
 *     :loading="loading"
 *     :total="total"
 *     v-model:page="page"
 *     @update:page="fetchData"
 *     @batch-delete="handleBatchDelete"
 *   />
 */
import { computed, ref, h } from 'vue'
import { NButton, NInput } from 'naive-ui'
import type { DataTableColumn, DataTableSortState, DataTableSize } from 'naive-ui'
import { debounce } from '@/utils/perform'

const props = withDefaults(
  defineProps<{
    columns: DataTableColumn[]
    data: Record<string, unknown>[]
    loading?: boolean
    total?: number
    page?: number
    pageSize?: number
    pageSizes?: number[]
    bordered?: boolean
    size?: DataTableSize
    singleLine?: boolean
    striped?: boolean
    maxHeight?: string | number
    flexHeight?: boolean
    rowKey?: string | ((row: Record<string, unknown>) => string | number)
    showPagination?: boolean
    showSizePicker?: boolean
    showQuickJumper?: boolean
    showToolbar?: boolean
    showSearch?: boolean
    searchPlaceholder?: string
    /** 多选：已选行 key 数组 */
    checkedRowKeys?: (string | number)[]
    /** 批量操作配置 */
    batchActions?: { delete?: boolean }
    /** 是否可行编辑 */
    editable?: boolean
    /** 排序状态 */
    sortState?: DataTableSortState | null
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
    showToolbar: true,
    showSearch: false,
    searchPlaceholder: '搜索...',
    checkedRowKeys: () => [],
    batchActions: () => ({ delete: false }),
    editable: false,
    sortState: null,
  },
)

const emit = defineEmits<{
  'update:page': [page: number]
  'update:page-size': [size: number]
  'update:checked-row-keys': [keys: (string | number)[]]
  'update:sort-state': [state: DataTableSortState | null]
  'batchDelete': [keys: (string | number)[]]
  'search': [keyword: string]
}>()

const tableRef = ref()
const searchKeyword = ref('')
const checkedRowKeys = ref<(string | number)[]>(props.checkedRowKeys)

const currentPage = computed({
  get: () => props.page,
  set: (val: number) => emit('update:page', val),
})

const pageCount = computed(() => {
  if (props.total > 0 && props.pageSize > 0) return Math.ceil(props.total / props.pageSize)
  return 1
})

// 合并选择列
const mergedColumns = computed<DataTableColumn[]>(() => {
  const checkboxCol: DataTableColumn = {
    type: 'selection',
    width: 40,
    fixed: 'left',
  }
  return props.editable ? [checkboxCol, ...props.columns.map(addEditRender)] : [checkboxCol, ...props.columns]
})

function addEditRender(col: DataTableColumn): DataTableColumn {
  if (!props.editable) return col
  const colKey = (col as { key?: string }).key
  if (!colKey) return col
  return {
    ...col,
    render: (row: Record<string, unknown>) => {
      return h(NInput, {
        value: row[colKey] as string,
        size: 'small',
        onUpdateValue: (val: string) => { row[colKey] = val },
      })
    },
  } as DataTableColumn
}

function onPageChange(p: number) { emit('update:page', p) }
function onPageSizeChange(s: number) { emit('update:page-size', s) }

function onCheckChange(keys: (string | number)[]) {
  checkedRowKeys.value = keys
  emit('update:checked-row-keys', keys)
}

function onSortChange(sorter: DataTableSortState | null) {
  emit('update:sort-state', sorter)
}

const onSearch = debounce((val: string) => {
  emit('search', val)
}, 300)
</script>

<style scoped>
.enhanced-table {
  width: 100%;
}
.enhanced-table__toolbar {
  display: flex;
  align-items: center;
  margin-bottom: 12px;
}
.enhanced-table__pagination {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding: 12px 0;
  gap: 12px;
}
.enhanced-table__total {
  font-size: 13px;
  color: #888;
}
</style>
