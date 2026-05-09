<template>
  <n-form
    ref="formRef"
    :model="formModel"
    :rules="rules"
    :label-placement="labelPlacement"
    :label-width="labelWidth"
    :size="size"
    :style="formStyle"
  >
    <n-grid :cols="cols" :x-gap="gap" :y-gap="gap">
      <n-form-item-gi
        v-for="field in fields"
        :key="field.key"
        :span="field.span || 1"
        :label="field.label"
        :path="field.key"
        :rule="field.rule"
      >
        <!-- 自定义内容插槽 -->
        <slot :name="field.key" :field="field" :model="formModel">
          <!-- 输入框 -->
          <n-input
            v-if="field.type === 'input'"
            v-model:value="formModel[field.key]"
            :placeholder="field.placeholder"
            :clearable="field.clearable"
            :maxlength="field.maxlength"
          />
          <!-- 选择器 -->
          <n-select
            v-else-if="field.type === 'select'"
            v-model:value="formModel[field.key]"
            :options="field.options"
            :placeholder="field.placeholder"
            :clearable="field.clearable"
            :multiple="field.multiple"
          />
          <!-- 开关 -->
          <n-switch
            v-else-if="field.type === 'switch'"
            v-model:value="formModel[field.key]"
          />
          <!-- 日期选择 -->
          <n-date-picker
            v-else-if="field.type === 'date'"
            v-model:value="formModel[field.key]"
            :type="field.dateType || 'date'"
            :placeholder="field.placeholder"
            style="width:100%"
          />
        </slot>
      </n-form-item-gi>
    </n-grid>

    <!-- 操作按钮 -->
    <n-space v-if="showActions" justify="end" style="margin-top:16px">
      <slot name="actions" :loading="loading">
        <n-button @click="handleReset">重置</n-button>
        <n-button type="primary" :loading="loading" @click="handleSubmit">提交</n-button>
      </slot>
    </n-space>
  </n-form>
</template>

<script setup lang="ts">
/**
 * 通用表单组件
 *
 * 支持栅格布局、联动校验、自适应。
 *
 * 使用方式：
 *   <BaseForm
 *     :fields="fields"
 *     v-model="formData"
 *     :rules="rules"
 *     @submit="handleSubmit"
 *   />
 */
import { ref, computed, watch } from 'vue'
import type { FormInst, FormRules, FormItemRule } from 'naive-ui'

/** 表单字段定义 */
export interface FormField {
  key: string
  label: string
  type?: 'input' | 'select' | 'switch' | 'date'
  span?: number
  placeholder?: string
  clearable?: boolean
  maxlength?: number
  options?: { label: string; value: unknown }[]
  multiple?: boolean
  dateType?: 'date' | 'datetime' | 'daterange'
  rule?: FormItemRule | FormItemRule[]
}

const props = withDefaults(
  defineProps<{
    fields: FormField[]
    modelValue: Record<string, unknown>
    rules?: FormRules
    cols?: number
    gap?: number
    labelPlacement?: 'left' | 'top'
    labelWidth?: number | string
    size?: 'small' | 'medium' | 'large'
    showActions?: boolean
    loading?: boolean
  }>(),
  {
    cols: 2,
    gap: 16,
    labelPlacement: 'left',
    labelWidth: 'auto',
    size: 'medium',
    showActions: true,
    loading: false,
  },
)

const emit = defineEmits<{
  'update:modelValue': [value: Record<string, unknown>]
  submit: [value: Record<string, unknown>]
}>()

const formRef = ref<FormInst | null>(null)
const formModel = ref<Record<string, unknown>>({ ...props.modelValue })

const formStyle = computed(() => ({
  maxWidth: props.cols > 1 ? '100%' : '600px',
}))

watch(
  () => props.modelValue,
  (val) => { formModel.value = { ...val } },
  { deep: true },
)

function handleSubmit() {
  formRef.value?.validate((errors) => {
    if (!errors) {
      emit('update:modelValue', { ...formModel.value })
      emit('submit', { ...formModel.value })
    }
  })
}

function handleReset() {
  formModel.value = {}
  formRef.value?.restoreValidation()
}

</script>
