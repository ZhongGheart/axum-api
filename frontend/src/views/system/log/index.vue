<template>
  <div class="page-container">
    <n-page-header title="系统日志" subtitle="操作记录查询与导出">
      <template #extra>
        <n-button v-permission="PERM.LOG_EXPORT" @click="handleExport">导出 Excel</n-button>
      </template>
    </n-page-header>

    <!--
      保留策略如实说明（v0.14.0）。
      此前"日志会被定期清理"这件事只存在于服务端 stdout 的一行日志里，
      界面上一个字都没有：筛一个 3 个月前的日期范围，什么都查不到，
      而管理员无从判断这是"那天什么都没发生"还是"发生过但被清了"。
      两种解释导向完全相反的处置，这个歧义本身就是审计的失效。
    -->
    <n-alert
      v-if="retention"
      type="info"
      :bordered="false"
      style="margin-bottom:12px"
    >
      <div class="retention-line">
        <span>{{ retentionText }}</span>
        <span v-if="purgeText" class="retention-sub">{{ purgeText }}</span>
      </div>
    </n-alert>

    <!--
      筛到的范围早于现存最老一条时必须说清：空结果不代表"没发生过"，
      很可能只是那段数据已被清理掉。这个提示只在**真的会遮住数据**时出现。
    -->
    <n-alert
      v-if="rangeBelowRetention"
      type="warning"
      :bordered="false"
      style="margin-bottom:12px"
    >
      所选时间范围早于现存最老一条日志（{{ formatTime(retention!.oldest_log_at) }}），
      该区间内的日志可能已被保留策略清理，空结果不代表那段时间没有操作。
    </n-alert>

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
import { computed, ref, reactive, onMounted, h } from 'vue'
import { NTag } from 'naive-ui'
import type { DataTableColumn } from 'naive-ui'
import { auditApi } from '@/api/audit'
import type { AuditLogItem, AuditLogListParams, AuditRetentionInfo } from '@/api/audit'
import {
  describePurge,
  describeRetention,
  formatTime,
  rangeStartsBeforeOldest,
} from '@/utils/auditRetention'
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

// 保留策略。取不到时保持 `null`，界面**不显示任何说明**——
// 宁可不说，也不能显示一个猜出来的"保留 90 天"：
// 那正是本版要消灭的"界面说谎"。
const retention = ref<AuditRetentionInfo | null>(null)

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
  // v0.13.0：摘要就是"这次改了什么"。此前这一列根本不存在，
  // 于是审计只回答了"谁调了哪个接口"，管理员看到一条 DELETE
  // 却不知道删掉的是哪个角色——信息在库里，界面上却读不到
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
  {
    title: '变更摘要',
    key: 'result',
    width: 420,
    // 摘要可能是一长串权限码，直接铺开会把表格撑到没法横向滚动
    ellipsis: { tooltip: true },
    render(row: Record<string, unknown>) {
      const r = row as unknown as AuditLogItem
      // 只读操作没有摘要，显示一个明确的破折号而不是空白：
      // 空白分不清是"没记"还是"这一行本来就没内容"
      return r.result?.trim() ? r.result : '—'
    },
  },
]

// 文案与判定都在 `utils/auditRetention` 里：这两件事很容易在边界情况下
// 说错话，而放在 SFC 里就只能靠截图验——截图看不出措辞对不对
const retentionText = computed(() => describeRetention(retention.value))
const purgeText = computed(() => describePurge(retention.value))

/**
 * 所选范围是否早于现存最老一条
 *
 * 按**范围起点**判定，而不是"结果为空时才提示"：
 * 范围 [3 个月前, 今天] 在只剩 1 个月日志时仍会返回非空结果，
 * 但那 2 个月的数据一样是缺的。只在空结果时才提示的话，
 * 用户会拿到一份"看起来查到了"的子集，而这正是最难发现的漏查。
 */
const rangeBelowRetention = computed(() =>
  rangeStartsBeforeOldest(filters.range, retention.value?.oldest_log_at ?? null))

async function fetchRetention() {
  try {
    retention.value = await auditApi.retention()
  } catch {
    // 拿不到就不显示：宁可少说一句，不可说错
    retention.value = null
  }
}

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

onMounted(() => {
  fetchRetention()
  fetchLogs()
})
</script>

<style scoped>
.retention-line {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

/* 次要那行用次要颜色，不与主信息抢注意力，但也不能弱到读不清 */
.retention-sub {
  opacity: 0.75;
  font-size: 12px;
}
</style>
