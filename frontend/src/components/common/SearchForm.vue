<template>
  <div class="search-form" :style="{ marginBottom: '16px' }">
    <n-space align="center" wrap>
      <slot name="before" />

      <!-- 搜索输入框 -->
      <n-input
        v-if="showSearch"
        v-model:value="searchValue"
        :placeholder="searchPlaceholder"
        :clearable="true"
        style="width: 240px"
        @input="onSearchInput"
        @clear="onSearchClear"
      >
        <template #prefix>
          <n-icon><SearchIcon /></n-icon>
        </template>
      </n-input>

      <!-- 下拉筛选（动态插槽） -->
      <slot name="filters" />

      <!-- 操作按钮区 -->
      <slot name="actions" />

      <slot name="after" />
    </n-space>
  </div>
</template>

<script setup lang="ts">
/**
 * 通用搜索表单组件
 *
 * 提供搜索输入框（300ms 防抖）、下拉筛选插槽、操作按钮插槽。
 *
 * 使用方式：
 *   <SearchForm @search="handleSearch" @clear="handleClear">
 *     <template #filters>
 *       <n-select v-model:value="status" :options="statusOptions" style="width:150px" />
 *     </template>
 *     <template #actions>
 *       <n-button type="primary" @click="handleSearch">查询</n-button>
 *     </template>
 *   </SearchForm>
 */
import { ref } from 'vue'
import { SearchOutline as SearchIcon } from '@vicons/ionicons5'
import { debounce } from '@/utils/perform'

const props = withDefaults(
  defineProps<{
    /** 是否显示搜索输入框 */
    showSearch?: boolean
    /** 搜索框占位文本 */
    searchPlaceholder?: string
    /** 防抖延迟（毫秒） */
    debounceMs?: number
  }>(),
  {
    showSearch: true,
    searchPlaceholder: '请输入关键字搜索',
    debounceMs: 300,
  },
)

const emit = defineEmits<{
  search: [value: string]
  clear: []
}>()

const searchValue = ref('')

/** 防抖搜索（默认 300ms） */
const onSearchInput = debounce((value: string) => {
  emit('search', value)
}, props.debounceMs)

/** 清空搜索 */
function onSearchClear() {
  searchValue.value = ''
  emit('clear')
}
</script>
