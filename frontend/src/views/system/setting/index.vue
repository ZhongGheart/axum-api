<template>
  <div class="page-container">
    <n-page-header title="系统参数" subtitle="口令策略与登录防护的运行时配置">
      <template #extra>
        <n-button v-permission="PERM.SETTING_UPDATE" quaternary @click="handleRefreshCache">
          清理缓存
        </n-button>
      </template>
    </n-page-header>

    <!--
      参数清单由后端下发，前端不硬编码。
      取不到时显示这句话而不是空白页：空白页与「没有可配置项」长得一模一样，
      而管理员此时的正确动作是看后端日志，不是刷新。
    -->
    <n-alert v-if="loadFailed" type="error" :bordered="false" style="margin-bottom: 12px">
      系统参数加载失败。若参数表不可用，后端会回落到部署配置与代码默认值，
      此时在页面上改值不会生效——请先确认服务端的参数表与缓存是否正常。
    </n-alert>

    <n-spin :show="loading">
      <n-space vertical size="large">
        <n-card
          v-for="section in sections"
          :key="section.group"
          :title="section.title"
          size="small"
        >
          <template #header-extra>
            <span class="section-count">{{ section.items.length }} 项</span>
          </template>

          <n-space vertical size="large">
            <div v-for="item in section.items" :key="item.key" class="setting-row">
              <div class="setting-main">
                <div class="setting-head">
                  <span class="setting-name">{{ item.name }}</span>
                  <n-tag :type="sourceTagType(item.source)" size="tiny" :bordered="false">
                    {{ sourceLabel(item.source) }}
                  </n-tag>
                  <n-tag v-if="!item.is_default" size="tiny" :bordered="false" type="warning">
                    已改过
                  </n-tag>
                </div>
                <div class="setting-desc">{{ item.description }}</div>
                <!--
                  消费方必须显示出来。v0.16.0 的整版主题是「开关存在但无效果」，
                  而管理员唯一的自查手段就是能一眼看出这个值被哪段代码读。
                -->
                <div class="setting-consumed">生效于：{{ item.consumed_by }}</div>
              </div>

              <div class="setting-control">
                <n-switch
                  v-if="item.value_type === 'bool'"
                  :value="item.value === 'true'"
                  :disabled="!canUpdate"
                  @update:value="(v: boolean) => handleUpdate(item, v ? 'true' : 'false')"
                />
                <n-input-number
                  v-else
                  :value="Number(item.value)"
                  :min="item.min"
                  :max="item.max"
                  :disabled="!canUpdate"
                  style="width: 140px"
                  @update:value="(v: number | null) => v !== null && handleUpdate(item, String(v))"
                />

                <n-button
                  v-permission="PERM.SETTING_UPDATE"
                  size="tiny"
                  quaternary
                  :disabled="!canUpdate || (!item.admin_overridden && item.is_default)"
                  @click="handleReset(item)"
                >
                  复位
                </n-button>
              </div>
            </div>
          </n-space>
        </n-card>
      </n-space>
    </n-spin>

    <!--
      跨参数约束写在页面底部，而不是只写在单个参数的 description 里：
      口令最小/最大长度是**一对**，单看任一个都发现不了「两个一起设会互相卡住」。
    -->
    <n-alert type="info" :bordered="false" style="margin-top: 16px">
      <div class="note">
        <div>· 口令最小长度必须小于口令最大长度，否则任何口令都无法通过校验；后端会在写入前拒绝这种组合。</div>
        <div>· 抬高口令强度**不会**要求存量用户立刻改口令：复杂度只在设置口令时校验，不在登录时校验。</div>
        <div>· 启用「口令有效期」后，存量口令按其设置时刻起算，多数会被要求在下次登录时改密。这是预期行为，但请先想清楚。</div>
        <div>· 登录失败锁定阈值与计数窗口若由部署配置（环境变量）决定，未被管理员显式改过前，页面上显示的是部署配置的值。</div>
      </div>
    </n-alert>
  </div>
</template>

<script setup lang="ts">
/**
 * 系统参数页
 *
 * 页面形状完全由后端 `GET /api/admin/settings` 的返回决定：
 * 参数名、类型、取值范围、默认值、说明、消费方分组都在响应里。
 * **前端不维护参数清单**——两处各写一份时，后端加一个参数而前端没跟上，
 * 表现是「接口支持、界面没有」，管理员会以为参数没生效。
 */
import { computed, onMounted, ref } from 'vue'
import { settingApi } from '@/api/setting'
import type { SettingGroup, SettingItem, SettingSource } from '@/api/setting'
import { usePermissionsStore } from '@/stores/permissions'
import { PERM } from '@/constants/permission'
import { showConfirm, showError, showSuccess } from '@/utils/message'

const permissions = usePermissionsStore()

const items = ref<SettingItem[]>([])
const loading = ref(false)
const loadFailed = ref(false)

/** 与后端 `SettingGroup` 对应；未知分组会落到「其他」而不是被静默丢弃 */
const GROUP_TITLES: Record<SettingGroup, string> = {
  password: '口令策略',
  login: '登录防护',
  registration: '注册准入',
}

const sections = computed(() => {
  const order: SettingGroup[] = ['password', 'login', 'registration']
  const grouped = new Map<string, SettingItem[]>()
  for (const item of items.value) {
    const bucket = grouped.get(item.group)
    if (bucket) bucket.push(item)
    else grouped.set(item.group, [item])
  }
  const known = order
    .filter((g) => grouped.has(g))
    .map((g) => ({ group: g, title: GROUP_TITLES[g], items: grouped.get(g)! }))
  const unknown = [...grouped.entries()]
    .filter(([g]) => !(g in GROUP_TITLES))
    .map(([g, list]) => ({ group: g, title: `其他（${g}）`, items: list }))
  // 未知分组排在最后而不是丢弃：后端加了新分组而前端没跟上时，
  // 丢弃会让那个参数**从界面上消失**，而它仍然生效。
  return [...known, ...unknown]
})

/** 无修改权限时控件整体禁用，而不是只藏按钮——只藏按钮会留下一个能看不能改的界面 */
const canUpdate = computed(() => permissions.has(PERM.SETTING_UPDATE))

function sourceLabel(source: SettingSource): string {
  switch (source) {
    case 'admin':
      return '管理员设置'
    case 'env':
      return '部署配置'
    default:
      return '代码默认值'
  }
}

function sourceTagType(source: SettingSource): 'info' | 'success' | 'default' {
  switch (source) {
    case 'admin':
      return 'info'
    case 'env':
      return 'success'
    default:
      return 'default'
  }
}

async function fetchSettings(): Promise<void> {
  loading.value = true
  loadFailed.value = false
  try {
    items.value = await settingApi.list()
  } catch {
    items.value = []
    loadFailed.value = true
  } finally {
    loading.value = false
  }
}

/** 局部替换一个参数，避免整表重拉导致滚动位置与焦点丢失 */
function replaceItem(key: string, next: SettingItem): void {
  items.value = items.value.map((it) => (it.key === key ? next : it))
}

async function handleUpdate(item: SettingItem, rawValue: string): Promise<void> {
  if (rawValue === item.value) return
  // 取值非法（越界、与另一参数冲突）时后端会 400 并说明原因，
  // 所以这里不做前端预校验——**两套校验规则必然漂移**，
  // 而后端那条是裁决方。让它把话说清楚，界面原样呈现。
  try {
    const saved = await settingApi.update(item.key, rawValue)
    replaceItem(item.key, saved)
    showSuccess(`${item.name} 已更新为 ${saved.value}`)
  } catch {
    // 响应拦截器已弹出错因；这里把控件拨回原值，
    // 否则界面会停在一个**后端并没有采纳**的取值上
    await fetchSettings()
  }
}

async function handleReset(item: SettingItem): Promise<void> {
  const hint = item.source === 'env'
    ? '该参数当前由部署配置决定，复位会把控制权交还给环境变量。'
    : `将恢复为代码默认值 ${item.default}。`
  const ok = await showConfirm({ content: `确认复位「${item.name}」？${hint}` })
  if (!ok) return
  try {
    const saved = await settingApi.reset(item.key)
    replaceItem(item.key, saved)
    showSuccess(`${item.name} 已复位为 ${saved.value}`)
  } catch {
    await fetchSettings()
  }
}

async function handleRefreshCache(): Promise<void> {
  try {
    const msg = await settingApi.refreshCache()
    showSuccess(msg || '参数缓存已清理')
  } catch {
    showError('清理缓存失败')
  }
}

onMounted(fetchSettings)
</script>

<style scoped>
.page-container {
  padding: 16px;
}

.section-count {
  font-size: 12px;
  color: var(--text-tertiary, #999);
}

.setting-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

/* 窄屏下参数名与控件必须换行，否则控件会被挤出视口、点不到 */
@media (max-width: 720px) {
  .setting-row {
    flex-direction: column;
    align-items: stretch;
  }
}

.setting-main {
  min-width: 0;
  flex: 1;
}

.setting-head {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.setting-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
}

.setting-desc {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary, #666);
}

.setting-consumed {
  margin-top: 2px;
  font-size: 12px;
  color: var(--text-tertiary, #999);
}

.setting-control {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.note {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 12px;
  line-height: 1.7;
}
</style>
