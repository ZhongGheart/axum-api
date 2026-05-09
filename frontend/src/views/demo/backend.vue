<template>
  <div class="page-container">
    <n-page-header title="后端能力示例" subtitle="分页 / 导出 / 校验 / CRUD 模板" />

    <n-card title="1. Excel 导出测试" class="demo-card">
      <n-button type="primary" @click="handleExport">导出用户列表 (Excel)</n-button>
    </n-card>

    <n-card title="2. 通用分页查询" class="demo-card">
      <n-pagination
        v-model:page="page"
        :page-count="pageCount"
        @update:page="fetchTest"
      />
      <n-data-table
        :columns="columns"
        :data="testData"
        :loading="loading"
        class="demo-table"
      />
    </n-card>

    <n-card title="3. 参数校验测试" class="demo-card">
      <n-space>
        <n-input v-model:value="testUsername" placeholder="输入用户名测试校验" style="width:200px" />
        <n-button @click="testValidation">测试校验</n-button>
      </n-space>
      <p v-if="validationResult" style="margin-top:8px">{{ validationResult }}</p>
    </n-card>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { useMessage } from 'naive-ui'
import type { DataTableColumn } from 'naive-ui'
import http from '@/api/index'

const message = useMessage()

// ── Excel 导出 ────────────────────────────────────────────────

async function handleExport() {
  try {
    const response = await http.get('/admin/export/users', {
      responseType: 'blob',
    })
    // 响应拦截器对 blob 类型返回原始 AxiosResponse，需取 .data
    const blob = (response as unknown as { data: Blob }).data
    const url = window.URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = '用户列表.xlsx'
    a.click()
    window.URL.revokeObjectURL(url)
    message.success('导出成功')
  } catch {
    message.error('导出失败')
  }
}

// ── 分页测试 ──────────────────────────────────────────────────

const page = ref(1)
const pageCount = ref(1)
const loading = ref(false)
const testData = ref<Record<string, unknown>[]>([])

const columns: DataTableColumn[] = [
  { title: 'ID', key: 'id' },
  { title: '用户名', key: 'username' },
  { title: '邮箱', key: 'email' },
]

async function fetchTest() {
  loading.value = true
  try {
    const res = await http.get('/admin/users', {
      params: { page: page.value, page_size: 10 },
    })
    const data = res as unknown as { items: unknown[]; total: number; page: number; total_pages: number }
    testData.value = data.items as Record<string, unknown>[]
    pageCount.value = data.total_pages
  } catch {
    // handled
  } finally {
    loading.value = false
  }
}

// ── 校验测试 ──────────────────────────────────────────────────

const testUsername = ref('')
const validationResult = ref('')

async function testValidation() {
  try {
    await http.post('/admin/validate', { username: testUsername.value })
    validationResult.value = '✅ 校验通过'
  } catch (e) {
    validationResult.value = `❌ ${(e as Error).message}`
  }
}
</script>

<style scoped>
.demo-card { margin-bottom: 16px; }
.demo-table { margin-top: 12px; }
</style>
