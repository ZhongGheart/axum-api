<template>
  <div class="monitor-page">
    <n-page-header title="系统监控" subtitle="服务器状态 / 数据库 / Redis">
      <template #extra>
        <n-button :loading="loading" @click="fetchData">刷新</n-button>
      </template>
    </n-page-header>

    <!-- 错误提示 -->
    <n-alert v-if="errorMsg" type="error" closable @close="errorMsg = ''" style="margin-bottom:12px">
      {{ errorMsg }}
    </n-alert>

    <n-grid :cols="3" :x-gap="12" :y-gap="12">
      <n-gi>
        <n-card title="CPU">
          <n-progress type="circle" :percentage="Math.round(cpuUsage)" :stroke-width="10" :rail-color="railColor">
            <template #default>{{ cpuUsage.toFixed(1) }}%</template>
          </n-progress>
          <template #footer>核心数: {{ data?.system.cpu.core_count || '-' }}</template>
        </n-card>
      </n-gi>
      <n-gi>
        <n-card title="内存">
          <n-progress type="circle" :percentage="Math.round(memUsage)" :stroke-width="10" :rail-color="railColor"
            :color="memColor">
            <template #default>{{ memUsage.toFixed(1) }}%</template>
          </n-progress>
          <template #footer>{{ usedMem }} MB / {{ totalMem }} MB</template>
        </n-card>
      </n-gi>
      <n-gi>
        <n-card title="磁盘">
          <n-progress type="circle" :percentage="Math.round(diskUsage)" :stroke-width="10" :rail-color="railColor"
            :color="diskColor">
            <template #default>{{ diskUsage.toFixed(1) }}%</template>
          </n-progress>
          <template #footer>{{ usedDisk }} GB / {{ totalDisk }} GB</template>
        </n-card>
      </n-gi>
    </n-grid>

    <n-grid :cols="2" :x-gap="12" :y-gap="12" style="margin-top:12px">
      <n-gi>
        <n-card title="数据库">
          <n-descriptions :column="1" size="small">
            <n-descriptions-item label="状态">{{ data?.database.connected ? '🟢 已连接' : '🔴 断开' }}</n-descriptions-item>
            <n-descriptions-item label="活跃连接">{{ data?.database.active_connections }}</n-descriptions-item>
            <n-descriptions-item label="最大连接">{{ data?.database.max_connections }}</n-descriptions-item>
            <n-descriptions-item label="数据库大小">{{ dbSize }}</n-descriptions-item>
          </n-descriptions>
        </n-card>
      </n-gi>
      <n-gi>
        <n-card title="Redis">
          <n-descriptions :column="1" size="small">
            <n-descriptions-item label="状态">{{ data?.redis.connected ? '🟢 已连接' : '🔴 断开' }}</n-descriptions-item>
            <n-descriptions-item label="客户端数">{{ data?.redis.connected_clients }}</n-descriptions-item>
            <n-descriptions-item label="内存">{{ redisMem }}</n-descriptions-item>
            <n-descriptions-item label="执行命令">{{ data?.redis.total_commands_processed }}</n-descriptions-item>
          </n-descriptions>
        </n-card>
      </n-gi>
    </n-grid>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { monitorApi } from '@/api/monitor'
import type { MonitorSystemInfo } from '@/api/monitor'
import { useAppStore } from '@/stores/app'

const appStore = useAppStore()
const loading = ref(false)
const data = ref<MonitorSystemInfo | null>(null)
const errorMsg = ref('')
let timer: ReturnType<typeof setInterval> | null = null

// ── 主题色适配 ──────────────────────────────────────────────

const railColor = computed(() => (appStore.isDark ? '#333' : '#f0f0f0'))

const memColor = computed(() => {
  if (!data.value) return '#2080f0'
  const pct = data.value.system.memory.usage_percent
  return pct > 80 ? '#e8802a' : pct > 60 ? '#f0a020' : '#18a058'
})

const diskColor = computed(() => {
  const d = data.value?.system.disks?.[0]
  if (!d) return '#2080f0'
  return d.usage_percent > 90 ? '#d03050' : d.usage_percent > 75 ? '#e8802a' : '#18a058'
})

// ── CPU ─────────────────────────────────────────────────────

const cpuUsage = computed(() => data.value?.system.cpu.usage_percent ?? 0)

// ── 内存 ────────────────────────────────────────────────────

const memUsage = computed(() => data.value?.system.memory.usage_percent ?? 0)
const totalMem = computed(() => data.value?.system.memory.total_mb ?? 0)
const usedMem = computed(() => data.value?.system.memory.used_mb ?? 0)

// ── 磁盘 ────────────────────────────────────────────────────

const diskUsage = computed(() => data.value?.system.disks?.[0]?.usage_percent ?? 0)
const totalDisk = computed(() => data.value?.system.disks?.[0]?.total_gb ?? 0)
const usedDisk = computed(() => data.value?.system.disks?.[0]?.used_gb ?? 0)

// ── DB / Redis ──────────────────────────────────────────────

const dbSize = computed(() => {
  const mb = data.value?.database.database_size_mb
  return mb ? `${mb.toFixed(1)} MB` : '-'
})

const redisMem = computed(() => {
  const bytes = data.value?.redis.used_memory_bytes
  if (!bytes) return '-'
  return bytes > 1024 * 1024
    ? `${(bytes / 1024 / 1024).toFixed(1)} MB`
    : `${(bytes / 1024).toFixed(1)} KB`
})

// ── 数据加载 ────────────────────────────────────────────────

async function fetchData() {
  loading.value = true
  errorMsg.value = ''
  try {
    const res = await monitorApi.getSystem()
    data.value = res as unknown as MonitorSystemInfo
  } catch (e) {
    errorMsg.value = (e as Error).message || '加载失败'
    console.warn('[Monitor] 获取系统状态失败:', e)
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchData()
  timer = setInterval(fetchData, 5_000)
})

onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
})
</script>
