<template>
  <n-select
    v-bind="$attrs"
    :value="modelValue"
    :options="options"
    :loading="loading"
    :filterable="filterable"
    :clearable="clearable"
    :multiple="multiple"
    :placeholder="placeholder || `请选择${labelName || ''}`"
    @update:value="$emit('update:modelValue', $event)"
  />
</template>

<script setup lang="ts">
/**
 * 字典下拉选择组件
 *
 * 基于字典编码远程加载字典项，支持缓存、远程搜索、多选。
 *
 * 使用方式：
 *   <DictSelect code="gender" v-model="gender" />
 *   <DictSelect code="status" v-model="status" multiple />
 */
import { ref, watch, computed, onMounted } from 'vue'
import type { SelectOption } from 'naive-ui'
import { dictApi } from '@/api/dict'
import type { DictItemInfo } from '@/api/dict'

const props = withDefaults(
  defineProps<{
    /** 字典编码（如 gender, status） */
    code: string
    /** v-model 绑定值 */
    modelValue?: string | string[] | null
    /** 是否支持筛选 */
    filterable?: boolean
    /** 是否可清空 */
    clearable?: boolean
    /** 是否多选 */
    multiple?: boolean
    /** 自定义占位符 */
    placeholder?: string
    /** 显示名称（用于占位符） */
    labelName?: string
  }>(),
  { filterable: true, clearable: true, multiple: false },
)

const emit = defineEmits<{
  'update:modelValue': [value: string | string[] | null]
}>()

const loading = ref(false)
const cache = new Map<string, { items: DictItemInfo[]; timestamp: number }>()
const CACHE_TTL = 60000 // 1 分钟
const items = ref<DictItemInfo[]>([])

const options = computed<SelectOption[]>(() => {
  return items.value.map((it) => ({
    label: it.label,
    value: it.value,
    disabled: it.status !== 'enabled',
  }))
})

async function fetchItems() {
  const cached = cache.get(props.code)
  if (cached && Date.now() - cached.timestamp < CACHE_TTL) {
    items.value = cached.items
    return
  }
  loading.value = true
  try {
    const res = (await dictApi.getCachedDict(props.code)) as unknown as DictItemInfo[]
    items.value = res
    cache.set(props.code, { items: res, timestamp: Date.now() })
  } catch {
    items.value = []
  } finally {
    loading.value = false
  }
}

watch(() => props.code, fetchItems, { immediate: true })
</script>
