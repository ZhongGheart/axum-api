<template>
  <div class="page-container">
    <n-page-header title="系统日志" subtitle="操作记录查询与导出">
      <template #extra>
        <n-button v-permission="PERM.LOG_EXPORT" @click="handleExport">导出 Excel</n-button>
      </template>
    </n-page-header>

    <n-card>
      <!-- 筛选栏 -->
      <n-space style="margin-bottom:12px" align="center" wrap>
        <n-input v-model:value="filters.action" placeholder="操作" clearable style="width:150px" />
        <n-input v-model:value="filters.username" placeholder="用户名" clearable style="width:150px" />
        <n-select
          v-model:value="filters.status_code"
          :options="statusOptions"
          placeholder="状态码"
          clearable
          style="width:120px"
        />
        <n-date-picker
          v-model:value="filters.range"
          type="datetimerange"
          clearable
          start-placeholder="开始时间"
          end-placeholder="结束时间"
          style="width:340px"
        />
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
import type { AuditLogItem, AuditLogListParams } from '@/api/audit'
import { showSuccess, showError, showWarning } from '@/utils/message'
import { PERM } from '@/constants/permission'

const loading = ref(false)
const logList = ref<AuditLogItem[]>([])
const page = ref(1)
const pageSize = ref(20)
const pageCount = ref(1)

const filters = reactive({
  action: '',
  username: '',
  status_code: null as number | null,
  range: null as [number, number] | null,
})

const statusOptions = [
  { label: '200 成功', value: 200 },
  { label: '400 参数错', value: 400 },
  { label: '401 未登录', value: 401 },
  { label: '403 无权限', value: 403 },
  { label: '404 不存在', value: 404 },
  { label: '500 服务错', value: 500 },
]

/**
 * 把界面状态收敛成后端认识的查询参数
 *
 * 列表和导出**共用这一个函数**：原实现里两者各拼各的，
 * 导出那侧压根没带条件，于是"筛完再导出"导出的是全量。
 *
 * 空值一律不传——后端开了 `deny_unknown_fields`，
 * 拼错的字段会直接 400，而不是悄悄被忽略。
 */
function buildParams(): Omit<AuditLogListParams, 'page' | 'page_size'> {
  const params: Omit<AuditLogListParams, 'page' | 'page_size'> = {}
  if (filters.action.trim()) params.action = filters.action.trim()
  if (filters.username.trim()) params.username = filters.username.trim()
  if (filters.status_code !== null) params.status_code = filters.status_code
  if (filters.range) {
    const [from, to] = filters.range
    params.start_time = new Date(from).toISOString()
    params.end_time = new Date(to).toISOString()
  }
  return params
}

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
    const data = await auditApi.list({
      page: page.value,
      page_size: pageSize.value,
      ...buildParams(),
    })
    logList.value = data.items
    pageCount.value = data.total_pages
  } catch { /* */ } finally { loading.value = false }
}

function search() { page.value = 1; fetchLogs() }
function resetFilters() {
  filters.action = ''
  filters.username = ''
  filters.status_code = null
  filters.range = null
  page.value = 1
  fetchLogs()
}

async function handleExport() {
  try {
    const { blob, rowCount, truncated, maxRows } = await auditApi.exportLogs(buildParams())
    const url = window.URL.createObjectURL(blob)
    const a = document.createElement('a'); a.href = url; a.download = '操作日志.xlsx'; a.click()
    window.URL.revokeObjectURL(url)
    if (truncated) {
      // 上限本身不是问题，静默截断才是——必须让用户知道拿到的是子集
      showWarning(`已导出 ${rowCount} 条，达到单次上限 ${maxRows} 条，数据不完整，请缩小筛选范围`)
    } else {
      showSuccess(`导出成功，共 ${rowCount} 条`)
    }
  } catch { showError('导出失败') }
}

onMounted(fetchLogs)
</script>
