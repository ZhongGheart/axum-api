<template>
  <div class="page-container">
    <n-page-header title="字典组件示例" subtitle="DictSelect 使用演示" />

    <n-grid :cols="2" :x-gap="16">
      <n-gi>
        <n-card title="基础用法" size="small">
          <n-form label-placement="top">
            <n-form-item label="性别（字典编码: gender）" path="gender">
              <DictSelect code="gender" v-model:value="form.gender" />
            </n-form-item>
            <n-form-item label="状态（字典编码: status）" path="status">
              <DictSelect code="status" v-model:value="form.status" />
            </n-form-item>
            <n-form-item label="消息类型（字典编码: msg_type）" path="msgType">
              <DictSelect code="msg_type" v-model:value="form.msgType" />
            </n-form-item>
          </n-form>
          <n-divider />
          <n-collapse>
            <n-collapse-item title="当前值">
              <pre>{{ JSON.stringify(form, null, 2) }}</pre>
            </n-collapse-item>
          </n-collapse>
        </n-card>
      </n-gi>
      <n-gi>
        <n-card title="高级用法" size="small">
          <n-form label-placement="top">
            <n-form-item label="多选模式" path="tags">
              <DictSelect code="tag" v-model:value="form.tags" multiple label-name="标签" />
            </n-form-item>
            <n-form-item label="清空 + 禁用选项" path="priority">
              <DictSelect code="priority" v-model:value="form.priority" clearable />
            </n-form-item>
            <n-form-item label="自定义占位符" path="custom">
              <DictSelect code="custom_type" v-model:value="form.custom" placeholder="请选择自定义类型" />
            </n-form-item>
          </n-form>
          <n-divider />
          <n-collapse>
            <n-collapse-item title="当前值">
              <pre>{{ JSON.stringify(form, null, 2) }}</pre>
            </n-collapse-item>
          </n-collapse>
        </n-card>
      </n-gi>
    </n-grid>
  </div>
</template>

<script setup lang="ts">
import { reactive } from 'vue'
import DictSelect from '@/components/common/DictSelect.vue'

const form = reactive({
  gender: null,
  status: null,
  msgType: null,
  tags: [],
  priority: null,
  custom: null,
})
</script>
