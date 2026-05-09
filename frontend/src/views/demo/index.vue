<template>
  <div class="demo-page">
    <n-page-header title="组件示例" subtitle="通用业务组件使用展示" />

    <!-- ── 图表组件 ──────────────────────────────────────── -->
    <n-card title="BaseChart 图表组件" class="demo-card">
      <n-grid :cols="2" :x-gap="16">
        <n-gi>
          <n-h4>折线图</n-h4>
          <BaseChart :options="lineOptions" :height="280" />
        </n-gi>
        <n-gi>
          <n-h4>柱状图</n-h4>
          <BaseChart :options="barOptions" :height="280" />
        </n-gi>
      </n-grid>
    </n-card>

    <!-- ── 表单组件 ──────────────────────────────────────── -->
    <n-card title="BaseForm 表单组件（栅格布局 + 联动校验）" class="demo-card">
      <BaseForm
        :fields="formFields"
        v-model="formData"
        :rules="formRules"
        :cols="2"
        @submit="onFormSubmit"
      />
    </n-card>

    <!-- ── 增强表格 ──────────────────────────────────────── -->
    <n-card title="EnhancedBaseTable 增强表格（多选 + 排序 + 批量操作）" class="demo-card">
      <EnhancedBaseTable
        :columns="tableColumns"
        :data="tableData"
        :total="tableData.length"
        :page="1"
        :page-size="10"
        :show-search="true"
        :batch-actions="{ delete: true }"
        @batch-delete="onBatchDelete"
        @search="onTableSearch"
      />
    </n-card>

    <!-- ── 弹窗组件 ──────────────────────────────────────── -->
    <n-card title="BaseModal 弹窗演示" class="demo-card">
      <n-space>
        <n-button type="primary" @click="showModal = true">打开弹窗</n-button>
        <n-button @click="showDraggable = true">拖拽弹窗</n-button>
      </n-space>
      <BaseModal v-model:show="showModal" title="普通弹窗" @ok="showModal = false">
        <p>这是一个普通弹窗的内容区域。</p>
      </BaseModal>
      <BaseModal v-model:show="showDraggable" title="可拖拽弹窗" draggable @ok="showDraggable = false">
        <p>拖拽顶部手柄可移动弹窗位置。</p>
      </BaseModal>
    </n-card>

    <!-- ── 上传组件 ──────────────────────────────────────── -->
    <n-card title="BaseUpload 上传组件" class="demo-card">
      <BaseUpload
        :action="'/api/upload'"
        mode="drag"
        accept="image/*"
        @success="onUploadSuccess"
      />
    </n-card>
  </div>
</template>

<script setup lang="ts">
/**
 * 组件示例页面
 *
 * 展示通用业务组件的使用方式。
 */
import { ref } from 'vue'
import { useMessage } from 'naive-ui'
import type { FormRules, DataTableColumn } from 'naive-ui'
import { BaseChart, BaseForm, BaseModal, BaseUpload, EnhancedBaseTable } from '@/components/common'
import type { FormField } from '@/components/common'

const message = useMessage()

// ── 弹窗状态 ────────────────────────────────────────────────

const showModal = ref(false)
const showDraggable = ref(false)

// ── 表单 ────────────────────────────────────────────────────

const formFields: FormField[] = [
  { key: 'name', label: '姓名', type: 'input', placeholder: '请输入姓名', rule: { required: true, message: '请输入姓名' } },
  { key: 'gender', label: '性别', type: 'select', options: [{ label: '男', value: 1 }, { label: '女', value: 2 }], placeholder: '请选择' },
  { key: 'email', label: '邮箱', type: 'input', placeholder: '请输入邮箱', span: 2 },
  { key: 'active', label: '启用', type: 'switch' },
  { key: 'birth', label: '出生日期', type: 'date', placeholder: '选择日期' },
]

const formData = ref<Record<string, unknown>>({})
const formRules: FormRules = {
  name: [{ required: true, message: '请输入姓名', trigger: 'blur' }],
  email: [{ type: 'email', message: '邮箱格式不正确', trigger: 'blur' }],
}

function onFormSubmit(values: Record<string, unknown>) {
  message.success('表单提交: ' + JSON.stringify(values))
}

// ── 图表 ────────────────────────────────────────────────────

const months = ['1月', '2月', '3月', '4月', '5月', '6月']

const lineOptions = {
  xAxis: { type: 'category' as const, data: months },
  yAxis: { type: 'value' as const },
  series: [{ type: 'line' as const, data: [120, 200, 150, 80, 70, 110], smooth: true }],
}

const barOptions = {
  xAxis: { type: 'category' as const, data: months },
  yAxis: { type: 'value' as const },
  series: [{ type: 'bar' as const, data: [30, 45, 60, 35, 50, 40], itemStyle: { color: '#2080f0' } }],
}

// ── 表格 ────────────────────────────────────────────────────

const tableData = Array.from({ length: 20 }, (_, i) => ({
  id: i + 1,
  name: `用户${i + 1}`,
  age: 20 + (i % 30),
  role: i % 3 === 0 ? '管理员' : '普通用户',
  status: i % 2 === 0,
}))

const tableColumns: DataTableColumn[] = [
  { title: 'ID', key: 'id', width: 60, sorter: true },
  { title: '姓名', key: 'name', width: 120 },
  { title: '年龄', key: 'age', width: 80, sorter: true },
  { title: '角色', key: 'role', width: 100 },
  { title: '状态', key: 'status', width: 80 },
]

function onBatchDelete(keys: (string | number)[]) {
  message.warning(`批量删除 IDs: ${keys.join(', ')}`)
}

function onTableSearch(keyword: string) {
  message.info(`搜索: ${keyword}`)
}

// ── 上传 ────────────────────────────────────────────────────

function onUploadSuccess() {
  message.success('上传成功')
}
</script>

<style scoped>
.demo-page {
  padding: 16px;
}
.demo-card {
  margin-bottom: 16px;
}
</style>
