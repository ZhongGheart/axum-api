<template>
  <n-layout position="absolute" has-sider class="app-layout">
    <!-- 侧栏 -->
    <n-layout-sider
      bordered
      :collapsed="appStore.collapsed"
      collapse-mode="width"
      :collapsed-width="64"
      :width="232"
      :native-scrollbar="false"
      class="layout-sider"
    >
      <!-- Logo：图标 + 名称，折叠时只剩图标 -->
      <div class="sider-logo" @click="router.push('/')">
        <span class="logo-mark">
          <n-icon size="18"><FlashIcon /></n-icon>
        </span>
        <transition name="fade">
          <span v-show="!appStore.collapsed" class="logo-text">Axum Admin</span>
        </transition>
      </div>

      <!-- 导航菜单 -->
      <n-menu
        :collapsed="appStore.collapsed"
        :collapsed-width="64"
        :collapsed-icon-size="20"
        :options="filteredMenu"
        :value="activeMenu"
        :render-label="renderMenuLabel"
        @update:value="onMenuSelect"
      />
    </n-layout-sider>

    <!-- 主区域 -->
    <n-layout class="main-layout">
      <!-- 顶栏 -->
      <n-layout-header bordered class="layout-header">
        <div class="header-left">
          <n-tooltip trigger="hover" placement="bottom">
            <template #trigger>
              <n-button quaternary size="small" :aria-label="collapsedHint" @click="appStore.toggleCollapsed">
                <template #icon>
                  <n-icon><MenuIcon /></n-icon>
                </template>
              </n-button>
            </template>
            {{ collapsedHint }}
          </n-tooltip>

          <n-breadcrumb class="header-breadcrumb">
            <n-breadcrumb-item v-for="(crumb, index) in breadcrumbs" :key="crumb.key">
              <span :class="['crumb', { 'crumb--current': index === breadcrumbs.length - 1 }]">
                {{ crumb.label }}
              </span>
            </n-breadcrumb-item>
          </n-breadcrumb>
        </div>

        <div class="header-right">
          <!-- 主题切换：图标按钮必须带 tooltip，否则亮/暗两个图标只能靠记忆区分 -->
          <n-tooltip trigger="hover" placement="bottom">
            <template #trigger>
              <n-button quaternary size="small" @click="appStore.toggleTheme">
                <template #icon>
                  <n-icon><MoonIcon v-if="!appStore.isDark" /><SunnyIcon v-else /></n-icon>
                </template>
              </n-button>
            </template>
            切换到{{ appStore.isDark ? '亮色' : '暗色' }}模式
          </n-tooltip>

          <!-- 用户信息 -->
          <n-dropdown :options="userMenuOptions" placement="bottom-end" @select="onUserMenuSelect">
            <div class="user-info" tabindex="0" role="button" @keyup.enter="onUserMenuSelect('profile')">
              <n-avatar round :size="30" :src="avatarSrc || undefined" :color="null">
                {{ userInitial }}
              </n-avatar>
              <div v-show="!appStore.collapsed" class="user-meta">
                <span class="user-name">{{ userDisplayName }}</span>
                <span v-if="roleLabel" class="user-role">{{ roleLabel }}</span>
              </div>
              <n-icon v-show="!appStore.collapsed" size="14" class="user-caret">
                <ChevronDownIcon />
              </n-icon>
            </div>
          </n-dropdown>
        </div>
      </n-layout-header>

      <!-- 内容区 -->
      <n-layout-content class="layout-content" :native-scrollbar="false">
        <router-view v-slot="{ Component }">
          <transition name="fade" mode="out-in">
            <component :is="Component" />
          </transition>
        </router-view>
      </n-layout-content>
    </n-layout>
  </n-layout>
</template>

<script setup lang="ts">
/**
 * 主布局组件
 *
 * 提供侧栏导航、顶栏（用户信息/主题切换/折叠按钮）、面包屑、内容区。
 */
import { computed, h, ref, watch } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { NIcon } from 'naive-ui'
import {
  MenuOutline as MenuIcon,
  SunnyOutline as SunnyIcon,
  MoonOutline as MoonIcon,
  FlashOutline as FlashIcon,
  ChevronDownOutline as ChevronDownIcon,
  LogOutOutline as LogoutIcon,
  // 以下按"路径兜底图标"命名，与 PATH_ICONS 的语义一一对应
  HomeOutline as HomeIcon,
  GridOutline as GridIcon,
  PeopleOutline as PeopleIcon,
  PersonOutline as UserIcon,
  ShieldCheckmarkOutline as ShieldIcon,
  KeyOutline as KeyIcon,
  ListOutline as ListIcon,
  AlbumsOutline as AlbumsIcon,
  ClipboardOutline as ClipboardIcon,
  ServerOutline as ServerIcon,
  BarChartOutline as BarChartIcon,
  CodeSlashOutline as CodeSlashIcon,
  ConstructOutline as ToolIcon,
} from '@vicons/ionicons5'
import type { MenuOption } from 'naive-ui'
import { useAppStore } from '@/stores/app'
import { useUserStore } from '@/stores/user'
import { useMenuStore } from '@/stores/menu'
import { resolveAvatarUrl } from '@/utils/avatar'
import { displayLabel } from '@/api/types/response'
import type { MenuNode } from '@/api/menu'
import { showConfirm } from '@/utils/message'

const router = useRouter()
const route = useRoute()
const appStore = useAppStore()
const userStore = useUserStore()
const menuStore = useMenuStore()

// 挂载时尝试获取用户信息（不阻塞渲染，失败也无影响）
userStore.fetchUserInfo().catch(() => {})

/** 用户头像（v0.20.0 上传的头像优先，回退首字母） */
const avatarSrc = computed(() => resolveAvatarUrl(userStore.userInfo?.avatar_url))

/** 用户头像回退：用户名首字母 */
const userInitial = computed(
  () => userStore.userInfo?.username?.charAt(0)?.toUpperCase() || 'U',
)

/** 用户展示名：优先展示名，回退用户名（与列表页同一套规则） */
const userDisplayName = computed(() => {
  const info = userStore.userInfo
  return info ? displayLabel(info) : '用户'
})

/** 角色标签：让用户一眼看出自己是什么权限，而不是只能靠猜 */
const roleLabel = computed(() => {
  const roles = userStore.userInfo?.roles
  if (roles && roles.length > 0) return roles.join('、')
  const role = userStore.userInfo?.role
  return role === 'admin' ? 'admin' : ''
})

/** 当前激活的菜单路径 */
const activeMenu = computed(() => route.path)

/** 折叠按钮的提示语要说清"点一下会发生什么" */
const collapsedHint = computed(() => (appStore.collapsed ? '展开侧边栏' : '收起侧边栏'))

// ── 面包屑 ──────────────────────────────────────────────────
//
// 此前只有一级 `route.meta.title`，于是从"用户管理"点进某个用户的
// 在线会话抽屉（地址变了、标题没变）时，用户看不出自己在哪。
// 现在按当前路径去后端菜单树里回溯祖先链，父级 + 当前页一起显示。

interface Crumb {
  key: string
  label: string
}

const breadcrumbs = ref<Crumb[]>([{ key: 'home', label: '首页' }])

/** 在菜单树里按路径找节点，返回从根到该节点的路径 */
function findTrail(nodes: MenuNode[], path: string, trail: MenuNode[]): MenuNode[] | null {
  for (const node of nodes) {
    const next = [...trail, node]
    if (node.path === path) return next
    if (node.children?.length) {
      const hit = findTrail(node.children, path, next)
      if (hit) return hit
    }
  }
  return null
}

function updateBreadcrumbs() {
  // 菜单可能尚未加载完，或当前路由压根不在菜单里（个人中心）。
  // 这两种情况下退回 meta 标题，不让面包屑空掉。
  const trail = menuStore.loaded ? findTrail(menuStore.menus, route.path, []) : null
  if (trail && trail.length > 0) {
    // 菜单树根节点通常就是当前这一页（如"用户管理"既是父级也是自己），
    // 这种情况下只显示一项，避免出现"用户管理 > 用户管理"。
    const items = trail
      .filter((node) => node.type !== 'catalog')
      .map((node) => ({ key: `${node.id}`, label: node.name }))
    if (items.length > 0) {
      breadcrumbs.value = items
      return
    }
  }
  breadcrumbs.value = [
    { key: 'root', label: '首页' },
    { key: 'current', label: (route.meta?.title as string) || '首页' },
  ]
}

watch(() => [route.path, menuStore.loaded], updateBreadcrumbs, { immediate: true })

// ── 导航菜单 ──────────────────────────────────────────────────

function renderIcon(icon: unknown) {
  return () => h(NIcon, null, { default: () => h(icon as never) })
}

/**
 * 菜单图标映射
 *
 * 此前只有 5 个键，且未识别的一律回退到齿轮——系统里于是出现七八个
 * 一模一样的齿轮，"监控"和"菜单管理"在侧栏上完全分不开。
 * 这里按后端菜单库里实际使用的 icon 名补齐，并保留分组图标。
 */
const MENU_ICONS: Record<string, unknown> = {
  home: HomeIcon,
  dashboard: GridIcon,
  grid: GridIcon,
  user: PeopleIcon,
  people: PeopleIcon,
  role: ShieldIcon,
  shield: ShieldIcon,
  permission: KeyIcon,
  key: KeyIcon,
  menu: ListIcon,
  list: ListIcon,
  dict: AlbumsIcon,
  album: AlbumsIcon,
  log: ClipboardIcon,
  clipboard: ClipboardIcon,
  monitor: ServerIcon,
  server: ServerIcon,
  api: BarChartIcon,
  chart: BarChartIcon,
  docs: CodeSlashIcon,
  'api-docs': CodeSlashIcon,
  code: CodeSlashIcon,
  tool: ToolIcon,
  tools: ToolIcon,
  setting: ToolIcon,
  settings: ToolIcon,
  config: ToolIcon,
}

/**
 * 路径 → 图标兜底
 *
 * 后端种子数据给「菜单管理 / 系统日志 / 接口文档 / 系统监控 / 接口监控 / 字典管理」
 * 六个条目都填了同一个 `settings`，于是侧栏上出现六排一模一样的齿轮，
 * 只能靠文字区分。前端按路径补一张兜底表把这些条目区分开——
 * 改的是**显示**，不动菜单表里的 icon 字段（那是管理员的数据）。
 */
const PATH_ICONS: Record<string, unknown> = {
  '/demo': GridIcon,
  '/demo/backend': ServerIcon,
  '/demo/dict': AlbumsIcon,
  '/system/user': PeopleIcon,
  '/system/role': ShieldIcon,
  '/system/menu': ListIcon,
  '/system/log': ClipboardIcon,
  '/system/api-docs': CodeSlashIcon,
  '/system/monitor/system': ServerIcon,
  '/system/monitor/api': BarChartIcon,
  '/system/dict': AlbumsIcon,
}

/** 菜单图标决策顺序：显式且具体的图标名 → 路径兜底 → 通用工具图标 */
function resolveMenuIcon(node: MenuNode): unknown {
  const name = node.icon ?? ''
  const byName = MENU_ICONS[name]
  // `settings` 过于笼统：八个条目都用它当占位，区分度为零，走路径兜底
  if (byName && name !== 'settings') return byName
  const byPath = PATH_ICONS[node.path ?? '']
  if (byPath) return byPath
  return byName ?? ToolIcon
}

/** 菜单树 → naive-ui 导航选项（导航完全由后端菜单驱动） */
function toMenuOptions(nodes: MenuNode[]): MenuOption[] {
  return nodes
    .filter((node) => Boolean(node.path))
    .map((node) => {
      const children = node.children?.length ? toMenuOptions(node.children) : []
      return {
        label: node.name,
        key: node.path as string,
        icon: renderIcon(resolveMenuIcon(node)),
        children: children.length > 0 ? children : undefined,
      }
    })
}

const filteredMenu = computed(() => toMenuOptions(menuStore.menus))

/**
 * 菜单项文案渲染
 *
 * 折叠态下 naive-ui 只画图标，此时给它补一个原生 title——否则折叠态的
 * 菜单项既没有 tooltip 也没有文字，用户只能一个个悬停试。
 *
 * **这个函数本身就是 naive-ui 调用的渲染器**，必须直接返回值。
 * 写成"返回一个函数"（`() => label`）时 naive-ui 会把那个函数本身
 * 当成要渲染的内容，侧栏于是显示出函数源码文本。
 */
function renderMenuLabel(option: { label?: string }) {
  const label = (option.label as string) ?? ''
  if (!appStore.collapsed) return label
  return h('span', { title: label }, label)
}

function onMenuSelect(key: string) {
  router.push(key)
}

// ── 用户下拉菜单 ──────────────────────────────────────────────

const userMenuOptions = [
  {
    label: '个人中心',
    key: 'profile',
    icon: renderIcon(UserIcon),
  },
  {
    type: 'divider' as const,
    key: 'divider',
  },
  {
    label: '退出登录',
    key: 'logout',
    icon: renderIcon(LogoutIcon),
  },
]

async function onUserMenuSelect(key: string) {
  if (key === 'profile') {
    router.push('/profile')
    return
  }
  if (key === 'logout') {
    const confirmed = await showConfirm({ content: '确定要退出登录吗？' })
    if (confirmed) {
      userStore.logout()
    }
  }
}
</script>

<style scoped>
.layout-sider {
  background: var(--bg-card);
}

/*
 * Logo 区：图标是一个带主色底的方块，不是 emoji。
 * emoji 在不同系统上字形不同（有的带彩色背景），用它当品牌标识
 * 意味着同一份代码在不同机器上长得不一样。
 */
.sider-logo {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 52px;
  padding: 0 16px;
  cursor: pointer;
  border-bottom: 1px solid var(--border-color);
  overflow: hidden;
}

.logo-mark {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 28px;
  height: 28px;
  border-radius: var(--radius-md);
  background: var(--primary-color);
  color: #fff;
}

.logo-text {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-primary);
  white-space: nowrap;
}

.layout-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 16px;
  height: 52px;
  background: var(--bg-card);
}

.header-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.header-breadcrumb {
  font-size: 13px;
  min-width: 0;
}

.crumb {
  color: var(--text-secondary);
}

.crumb--current {
  color: var(--text-primary);
  font-weight: 500;
}

.header-right {
  display: flex;
  align-items: center;
  gap: 4px;
}

.user-info {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  padding: 4px 8px;
  border-radius: var(--radius-md);
  transition: background 0.15s;
}

/* 此前写死 rgba(0,0,0,0.05)，在暗色主题下几乎看不出变化 */
.user-info:hover {
  background: var(--bg-hover);
}

.user-meta {
  display: flex;
  flex-direction: column;
  line-height: 1.25;
  min-width: 0;
}

.user-name {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-primary);
  max-width: 140px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.user-role {
  font-size: 11px;
  color: var(--text-tertiary);
  max-width: 140px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.user-caret {
  color: var(--text-tertiary);
}

.layout-content {
  padding: 16px;
  min-height: calc(100vh - 52px);
  background: var(--bg-color);
}
</style>