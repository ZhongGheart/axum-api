<template>
  <div class="monitor-page">
    <n-page-header title="系统监控" subtitle="服务器状态 / 数据库 / Redis">
      <template #extra>
        <n-button :loading="loading" @click="fetchData">刷新</n-button>
      </template>
    </n-page-header>

    <n-grid :cols="3" :x-gap="12" :y-gap="12">
      <!-- CPU -->
      <n-gi>
        <n-card title="CPU">
          <n-progress type="circle" :percentage="Math.round(cpuUsage)" :stroke-width="10" :rail-color="railColor">
            <template #default>{{ cpuUsage.toFixed(1) }}%</template>
          </n-progress>
          <template #footer>核心数: {{ sysInfo?.cpu.core_count || '-' }}</template>
        </n-card>
      </n-gi>
      <!-- 内存 -->
      <n-gi>
        <n-card title="内存">
          <n-progress type="circle" :percentage="Math.round(memUsage)" :stroke-width="10" :rail-color="railColor"
            :color="memColor">
            <template #default>{{ memUsage.toFixed(1) }}%</template>
          </n-progress>
          <template #footer>{{ usedMem }} MB / {{ totalMem }} MB</template>
        </n-card>
      </n-gi>
      <!-- 磁盘 -->
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
      <!-- 数据库 -->
      <n-gi>
        <n-card title="数据库">
          <n-description>
            <n-description-item label="状态">{{ dbStatus ? '🟢 已连接' : '🔴 断开' }}</n-description-item>
            <n-description-item label="活跃连接">{{ sysInfo?.database.active_connections }}</n-description-item>
            <n-description-item label="最大连接">{{ sysInfo?.database.max_connections }}</n-description-item>
            <n-description-item label="数据库大小">{{ sysInfo?.database.database_size_mb.toFixed(1) }} MB</n-description-item>
          </n-description>
        </n-card>
      </n-gi>
      <!-- Redis -->
      <n-gi>
        <n-card title="Redis">
          <n-description>
            <n-description-item label="状态">{{ redisConnected ? '🟢 已连接' : '🔴 断开' }}</n-description-item>
            <n-description-item label="客户端数">{{ sysInfo?.redis.connected_clients }}</n-description-item>
            <n-description-item label="内存">{{ redisMem }}</n-description-item>
            <n-description-item label="执行命令">{{ sysInfo?.redis.total_commands_processed }}</n-description-item>
          </n-description>
        </n-card>
      </n-gi>
    </n-grid>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { monitorApi } from '@/api/monitor'
import type { SystemInfo } from '@/api/monitor'
import { useAppStore } from '@/stores/app'

const appStore = useAppStore()
const loading = ref(false)
const sysInfo = ref<SystemInfo | null>(null)
let timer: ReturnType<typeof setInterval> | null = null

const railColor = computed(() => appStore.isDark ? '#333' : '#f0f0f0')
const memColor = computed(() => sysInfo.value && sysInfo.value.memory.usage_percent > 80 ? '#e8802a' : '#18a058')
const diskColor = computed(() => {
  if (!sysInfo.value || sysInfo.value.disks.length === 0) return '#2080f0'
  const d = sysInfo.value.disks[0]
  return d.usage_percent > 90 ? '#d03050' : d.usage_percent > 75 ? '#e8802a' : '#18a058'
})

const cpuUsage = computed(() => sysInfo.value?.system.cpu.usage_percent ?? 0)
const memUsage = computed(() => sysInfo.value?.memory.usage_percent ?? 0)
const totalMem = computed(() => sysInfo.value?.memory.total_mb ?? 0)
const usedMem = computed(() => sysInfo.value?.memory.used_mb ?? 0)
const diskUsage = computed(() => sysInfo.value?.disks[0]?.usage_percent ?? 0)
const totalDisk = computed(() => sysInfo.value?.disks[0]?.total_gb ?? 0)
const usedDisk = computed(() => sysInfo.value?.disks[0]?.used_gb ?? 0)
const dbStatus = computed(() => sysInfo.value?.database.connected ?? false)
const redisConnected = computed(() => sysInfo.value?.redis.connected ?? false)
const redisMem = computed(() => {
  if (!sysInfo.value) return '-'
  const bytes = sysInfo.value.redis.used_memory_bytes
  return bytes > 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : `${(bytes / 1024).toFixed(1)} KB`
})

async function fetchData() {
  loading.value = true
  try { sysInfo.value = (await monitorApi.getSystem()) as unknown as SystemInfo }
  catch { /* */ }
  finally { loading.value = false }
}

onMounted(() => {
  fetchData()
  timer = setInterval(fetchData, 5000)
})

onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
})
</script>
