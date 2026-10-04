<template>
  <div class="dashboard">
    <!-- 页面头 -->
    <header class="dashboard-header">
      <div>
        <h1 class="dashboard-title">{{ greeting }}，{{ userName }}</h1>
        <p class="dashboard-subtitle">
          {{ subtitle }}
        </p>
      </div>
      <n-button size="small" :loading="loading" @click="loadAll">
        <template #icon><n-icon><RefreshIcon /></n-icon></template>
        刷新
      </n-button>
    </header>

    <!-- 加载失败：说明是哪个接口挂了，而不是笼统一句"加载失败" -->
    <n-alert
      v-if="fatalError"
      type="warning"
      closable
      class="dashboard-alert"
      @close="fatalError = ''"
    >
      {{ fatalError }}
    </n-alert>

    <!-- 指标卡：只渲染当前用户有权看的指标 -->
    <n-grid v-if="visibleStats.length > 0" cols="1 s:2 l:4" :x-gap="12" :y-gap="12" responsive="screen">
      <n-gi v-for="stat in visibleStats" :key="stat.key">
        <n-card class="stat-card" :class="{ 'stat-card--error': stat.failed }">
          <div class="stat">
            <span class="stat-icon" :style="{ background: stat.tintBg, color: stat.tintFg }">
              <n-icon size="18"><component :is="stat.icon" /></n-icon>
            </span>
            <div class="stat-body">
              <span class="stat-label">{{ stat.label }}</span>
              <!-- 取不到的指标显示"—"而不是 0：0 是个断言，"—"才是"我不知道" -->
              <span class="stat-value">{{ stat.failed ? '—' : stat.value }}</span>
              <span v-if="stat.hint" class="stat-hint">{{ stat.hint }}</span>
            </div>
          </div>
        </n-card>
      </n-gi>
    </n-grid>

    <!-- 无任何管理权限：这不是错误，普通用户本来就该看到这一屏 -->
    <n-empty v-else-if="!loading" description="当前账号没有可查看的系统概览，可从左侧菜单进入已授权的页面">
      <template #extra>
        <n-button size="small" @click="router.push('/profile')">前往个人中心</n-button>
      </template>
    </n-empty>

    <n-grid cols="1 l:2" :x-gap="12" :y-gap="12" responsive="screen" class="dashboard-body">
      <!-- 最近审计 -->
      <n-gi v-if="canViewLogs">
        <n-card title="最近操作" size="small" class="panel">
          <template #header-extra>
            <n-button v-if="logListPath" text size="small" @click="router.push(logListPath)">
              查看全部
              <template #icon><n-icon><ChevronForwardIcon /></n-icon></template>
            </n-button>
          </template>

          <n-spin :show="logsLoading">
            <n-alert v-if="logsError" type="error" size="small" class="panel-alert">
              {{ logsError }}
            </n-alert>
            <n-list v-else-if="recentLogs.length > 0" hoverable>
              <n-list-item v-for="log in recentLogs" :key="log.id">
                <n-thing>
                  <template #header>
                    <span class="log-action">{{ log.action }}</span>
                    <n-tag
                      size="small"
                      :bordered="false"
                      :type="log.status_code >= 500 ? 'error' : log.status_code >= 400 ? 'warning' : 'default'"
                    >
                      {{ log.status_code }}
                    </n-tag>
                  </template>
                  <template #description>
                    {{ log.username || '匿名' }} · {{ log.path }}
                  </template>
                  <template #header-extra>
                    <span class="log-time" :title="formatDateTime(log.created_at)">
                      {{ formatRelativeTime(log.created_at) }}
                    </span>
                  </template>
                </n-thing>
              </n-list-item>
            </n-list>
            <n-empty v-else size="small" description="暂无操作记录" />
          </n-spin>
        </n-card>
      </n-gi>

      <!-- 运行状态：整个面板只在有监控权限时出现。
           否则上面已经是"没有可查看的概览"，这里再来一张"无监控权限"的空卡
           等于把同一件事说两遍，页面显得更空而不是更清楚。 -->
      <n-gi v-if="canViewMonitor">
        <n-card title="运行状态" size="small" class="panel">
          <template #header-extra>
            <n-button
              v-if="canViewMonitor && monitorPath"
              text
              size="small"
              @click="router.push(monitorPath)"
            >
              监控详情
              <template #icon><n-icon><ChevronForwardIcon /></n-icon></template>
            </n-button>
          </template>

          <n-spin :show="monitorLoading">
            <n-alert v-if="monitorError" type="error" size="small" class="panel-alert">
              {{ monitorError }}
            </n-alert>
            <n-descriptions v-else-if="systemInfo" :column="1" size="small" label-placement="left">
              <n-descriptions-item label="数据库">
                <span class="status">
                  <span class="dot" :class="systemInfo.database.connected ? 'dot--ok' : 'dot--bad'" />
                  {{ systemInfo.database.connected ? '已连接' : '断开' }}
                </span>
              </n-descriptions-item>
              <n-descriptions-item label="缓存">
                <span class="status">
                  <span class="dot" :class="systemInfo.redis.connected ? 'dot--ok' : 'dot--bad'" />
                  {{ systemInfo.redis.connected ? '已连接' : '断开' }}
                </span>
              </n-descriptions-item>
              <n-descriptions-item label="运行时长">
                {{ formatDuration(systemInfo.system.uptime_seconds) }}
              </n-descriptions-item>
              <n-descriptions-item label="内存占用">
                {{ systemInfo.system.memory.usage_percent.toFixed(1) }}%
              </n-descriptions-item>
              <n-descriptions-item label="主机">
                {{ systemInfo.system.hostname }}
              </n-descriptions-item>
            </n-descriptions>
            <n-empty v-else size="small" description="暂无状态数据" />
          </n-spin>
        </n-card>
      </n-gi>
    </n-grid>
  </div>
</template>

<script setup lang="ts">
/**
 * 首页仪表盘
 *
 * 此前这一屏是 30 行占位卡片，登录后第一眼看到的是一句"欢迎使用"。
 * 现在展示四个可授权的指标、最近审计与运行状态。
 *
 * **权限降级是这一屏的核心约束**：普通用户没有 system:* 权限，
 * 如果照单全去请求，用户会在自己没有任何越权操作的情况下看到一屏 403。
 * 因此每个指标都先问 permissionsStore，再决定发不发请求；
 * 没权限的卡片根本不渲染，而不是渲染成 0 或报错。
 */
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import {
  PeopleOutline,
  ShieldCheckmarkOutline,
  DocumentTextOutline,
  ServerOutline,
  // 模板里用的是带 Icon 后缀的别名，导入名保持一致才不会触发未使用告警
  RefreshOutline as RefreshIcon,
  ChevronForwardOutline as ChevronForwardIcon,
} from '@vicons/ionicons5'
import { userApi } from '@/api/user'
import { roleApi } from '@/api/role'
import { auditApi, type AuditLogItem } from '@/api/audit'
import { monitorApi, type MonitorSystemInfo } from '@/api/monitor'
import { PERM } from '@/constants/permission'
import { useUserStore } from '@/stores/user'
import { usePermissionsStore } from '@/stores/permissions'
import { useMenuStore } from '@/stores/menu'
import type { MenuNode } from '@/api/menu'
import { displayLabel } from '@/api/types/response'
import { formatDateTime, formatDuration, formatRelativeTime } from '@/utils/time'

const router = useRouter()
const userStore = useUserStore()
const permissionsStore = usePermissionsStore()
const menuStore = useMenuStore()

const loading = ref(false)
const fatalError = ref('')

const totalUsers = ref<number | null>(null)
const usersFailed = ref(false)
const totalRoles = ref<number | null>(null)
const rolesFailed = ref(false)

const recentLogs = ref<AuditLogItem[]>([])
const logsLoading = ref(false)
const logsError = ref('')

const systemInfo = ref<MonitorSystemInfo | null>(null)
const monitorLoading = ref(false)
const monitorError = ref('')

// ── 跳转路径 ──────────────────────────────────────────────────
//
// 目标路径**不在这里写死**。菜单路径是管理员在菜单管理里可改的数据，
// 写死 `/monitor/system` 这种字面量意味着：一旦管理员改了路径，
// 仪表盘上的"查看全部"就会变成一条 404，而没有任何编译期或运行期提示。
//
// 这里按 component 标识去菜单树里反查当前真实路径；
// 查不到（管理员删了菜单 / 当前用户看不到）时返回 null，按钮就不渲染，
// 而不是渲染一个点了必然 404 的链接。

/** 按 component 标识在菜单树里找出该页面的路径 */
function findPathByComponent(nodes: MenuNode[], component: string): string | null {
  for (const node of nodes) {
    if (node.component === component && node.path) return node.path
    if (node.children?.length) {
      const hit = findPathByComponent(node.children, component)
      if (hit) return hit
    }
  }
  return null
}

const logListPath = computed(() => findPathByComponent(menuStore.menus, 'system/log/index'))
const monitorPath = computed(() => findPathByComponent(menuStore.menus, 'monitor/system/index'))

// ── 权限判定 ──────────────────────────────────────────────────

const canViewUsers = computed(() => permissionsStore.has(PERM.USER_LIST))
const canViewRoles = computed(() => permissionsStore.has(PERM.ROLE_LIST))
const canViewLogs = computed(() => permissionsStore.has(PERM.LOG_LIST))
const canViewMonitor = computed(() => permissionsStore.has(PERM.MONITOR_SYSTEM))

// ── 指标卡 ────────────────────────────────────────────────────

interface StatCard {
  key: string
  label: string
  value: string
  hint: string
  icon: unknown
  tintBg: string
  tintFg: string
  failed: boolean
}

const visibleStats = computed<StatCard[]>(() => {
  const cards: StatCard[] = []

  if (canViewUsers.value) {
    cards.push({
      key: 'users',
      label: '用户总数',
      value: totalUsers.value === null ? '—' : String(totalUsers.value),
      hint: usersFailed.value ? '加载失败' : '已注册账号',
      icon: PeopleOutline,
      tintBg: 'var(--primary-color-soft)',
      tintFg: 'var(--primary-color)',
      failed: usersFailed.value,
    })
  }

  if (canViewRoles.value) {
    cards.push({
      key: 'roles',
      label: '角色数量',
      value: totalRoles.value === null ? '—' : String(totalRoles.value),
      hint: rolesFailed.value ? '加载失败' : '可分配角色',
      icon: ShieldCheckmarkOutline,
      tintBg: 'var(--success-color-soft)',
      tintFg: 'var(--success-color)',
      failed: rolesFailed.value,
    })
  }

  if (canViewLogs.value) {
    const errorCount = recentLogs.value.filter((log) => log.status_code >= 400).length
    cards.push({
      key: 'logs',
      label: '最近操作',
      value: recentLogs.value.length > 0 ? String(recentLogs.value.length) : '—',
      hint: errorCount > 0 ? `其中 ${errorCount} 条失败` : '最近 8 条',
      icon: DocumentTextOutline,
      tintBg: errorCount > 0 ? 'var(--danger-color-soft)' : 'var(--warning-color-soft)',
      tintFg: errorCount > 0 ? 'var(--danger-color)' : 'var(--warning-color)',
      failed: logsError.value !== '',
    })
  }

  if (canViewMonitor.value && systemInfo.value) {
    cards.push({
      key: 'runtime',
      label: '运行状态',
      value: systemInfo.value.database.connected && systemInfo.value.redis.connected ? '正常' : '异常',
      hint: `已运行 ${formatDuration(systemInfo.value.system.uptime_seconds)}`,
      icon: ServerOutline,
      tintBg:
        systemInfo.value.database.connected && systemInfo.value.redis.connected
          ? 'var(--success-color-soft)'
          : 'var(--danger-color-soft)',
      tintFg:
        systemInfo.value.database.connected && systemInfo.value.redis.connected
          ? 'var(--success-color)'
          : 'var(--danger-color)',
      failed: monitorError.value !== '',
    })
  }

  return cards
})

// ── 数据加载 ──────────────────────────────────────────────────

/** 单个指标失败不应让整屏失败，所以各自吞掉错误并标记 failed */
async function loadUsers(): Promise<void> {
  if (!canViewUsers.value) return
  try {
    // 响应拦截器已把 ApiResponse 解包成 `data`，但 axios 的类型声明仍是
    // AxiosResponse<T>，这里按运行时对齐（与 api/audit.ts 同一处理）
    const res = (await userApi.list({ page: 1, page_size: 1 })) as unknown as { total: number }
    totalUsers.value = res.total
    usersFailed.value = false
  } catch (error) {
    usersFailed.value = true
    console.warn('[Dashboard] 用户总数加载失败:', error)
  }
}

async function loadRoles(): Promise<void> {
  if (!canViewRoles.value) return
  try {
    const res = await roleApi.list({ page: 1, page_size: 1 })
    totalRoles.value = res.total
    rolesFailed.value = false
  } catch (error) {
    rolesFailed.value = true
    console.warn('[Dashboard] 角色数加载失败:', error)
  }
}

async function loadLogs(): Promise<void> {
  if (!canViewLogs.value) return
  logsLoading.value = true
  logsError.value = ''
  try {
    const res = await auditApi.list({ page: 1, page_size: 8, sort_order: 'desc' })
    recentLogs.value = res.items ?? []
  } catch (error) {
    logsError.value = (error as Error).message || '加载失败'
    recentLogs.value = []
  } finally {
    logsLoading.value = false
  }
}

async function loadMonitor(): Promise<void> {
  if (!canViewMonitor.value) return
  monitorLoading.value = true
  monitorError.value = ''
  try {
    const res = await monitorApi.getSystem()
    systemInfo.value = res as unknown as MonitorSystemInfo
  } catch (error) {
    monitorError.value = (error as Error).message || '加载失败'
    systemInfo.value = null
  } finally {
    monitorLoading.value = false
  }
}

async function loadAll(): Promise<void> {
  loading.value = true
  fatalError.value = ''
  // 并发发起：四个接口互不依赖，串行会把首屏耗时叠成四倍
  await Promise.all([loadUsers(), loadRoles(), loadLogs(), loadMonitor()])
  loading.value = false
}

// ── 展示文案 ──────────────────────────────────────────────────

const userName = computed(() => {
  const info = userStore.userInfo
  return info ? displayLabel(info) : '用户'
})

/** 按当前时段给不同的问候语，比一句"欢迎使用"更像在跟人说话 */
const greeting = computed(() => {
  const hour = new Date().getHours()
  if (hour < 6) return '夜深了'
  if (hour < 12) return '早上好'
  if (hour < 14) return '中午好'
  if (hour < 18) return '下午好'
  return '晚上好'
})

const subtitle = computed(() => {
  const count = visibleStats.value.length
  if (loading.value && count === 0) return '正在加载系统概览...'
  if (count === 0) return '这里会显示你有权限查看的系统概览'
  return '系统运行概览与最近操作记录'
})

onMounted(loadAll)
</script>

<style scoped>
.dashboard {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.dashboard-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.dashboard-title {
  font-size: 20px;
  font-weight: 600;
  color: var(--text-primary);
  line-height: 1.3;
}

.dashboard-subtitle {
  margin-top: 4px;
  font-size: 13px;
  color: var(--text-secondary);
}

.dashboard-alert {
  margin: 0;
}

.stat-card :deep(.n-card__content) {
  padding: 16px 18px;
}

.stat {
  display: flex;
  align-items: center;
  gap: 12px;
}

.stat-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 36px;
  height: 36px;
  border-radius: var(--radius-md);
}

.stat-body {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.stat-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.stat-value {
  font-size: 20px;
  font-weight: 600;
  color: var(--text-primary);
  line-height: 1.3;
}

.stat-hint {
  font-size: 11px;
  color: var(--text-tertiary);
}

.dashboard-body {
  margin: 0;
}

.panel-alert {
  margin-bottom: 8px;
}

.log-action {
  font-weight: 500;
  color: var(--text-primary);
}

.log-time {
  font-size: 12px;
  color: var(--text-tertiary);
  white-space: nowrap;
}

.status {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

/* 用圆点而不是 🟢/🔴 emoji：emoji 在不同系统上字形与对齐全不一致 */
.dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex-shrink: 0;
}

.dot--ok {
  background: var(--success-color);
}

.dot--bad {
  background: var(--danger-color);
}
</style>
