<template>
  <div class="page-container">
    <n-page-header title="系统日志" subtitle="操作记录查询与导出">
      <template #extra>
        <n-button @click="handleExport">导出 Excel</n-button>
      </template>
    </n-page-header>

    <n-card>
      <!-- 筛选栏 -->
      <n-space style="margin-bottom:12px" align="center">
        <n-input v-model:value="filters.action" placeholder="操作" clearable style="width:150px" />
        <n-input v-model:value="filters.username" placeholder="用户名" clearable style="width:150px" />
        <n-button type="primary" @click="search">查询</n-button>
        <n-button @click="resetFilters">重置</n-button>
      </n-space>

      <n-data-table :columns="columns" :data="logList" :loading="loading" :bordered="true" size="small" />

      <n-pagination
        v-model:page="page"
        :page-count="pageCount"
        :page-size="pageSize"
        style="margin-top:12px;justify-content:flex-end"
        @update:page="fetchLogs"
      />
    </n-card>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, onMounted, h } from 'vue'
import { NTag } from 'naive-ui'
import type { DataTableColumn } from 'naive-ui'
import { auditApi } from '@/api/audit'
import type { AuditLogItem } from '@/api/audit'
import { showSuccess, showError } from '@/utils/message'
import { getToken } from '@/utils/storage'

const loading = ref(false)
const logList = ref<AuditLogItem[]>([])
const page = ref(1)
const pageSize = ref(20)
const pageCount = ref(1)

const filters = reactive({ action: '', username: '' })

const columns: DataTableColumn[] = [
  { title: '用户名', key: 'username', width: 100 },
  { title: '操作', key: 'action', width: 180 },
  { title: '方法', key: 'method', width: 80 },
  { title: '路径', key: 'path', width: 300, ellipsis: { tooltip: true } },
  { title: '状态码', key: 'status_code', width: 80,
    render(row: Record<string, unknown>) {
      const r = row as unknown as AuditLogItem
      const type = r.status_code >= 400 ? 'error' : r.status_code >= 300 ? 'warning' : 'success'
      return h(NTag, { type, size: 'small' }, () => String(r.status_code))
    },
  },
  { title: 'IP', key: 'client_ip', width: 140 },
  { title: '耗时(ms)', key: 'duration_ms', width: 80 },
  { title: '时间', key: 'created_at', width: 180 },
]

async function fetchLogs() {
  loading.value = true
  try {
    const res = await auditApi.list({ page: page.value, page_size: pageSize.value, ...filters })
    const data = res as unknown as { items: AuditLogItem[]; total: number; page: number; total_pages: number }
    logList.value = data.items
    pageCount.value = data.total_pages
  } catch { /* */ } finally { loading.value = false }
}

function search() { page.value = 1; fetchLogs() }
function resetFilters() { filters.action = ''; filters.username = ''; page.value = 1; fetchLogs() }

async function handleExport() {
  try {
    const token = getToken()
    const res = await fetch('/api/admin/logs/audit/export', {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
    })
    if (!res.ok) throw new Error('导出失败')
    const blob = await res.blob()
    const url = window.URL.createObjectURL(blob)
    const a = document.createElement('a'); a.href = url; a.download = '操作日志.xlsx'; a.click()
    window.URL.revokeObjectURL(url)
    showSuccess('导出成功')
  } catch { showError('导出失败') }
}

onMounted(fetchLogs)
</script>
