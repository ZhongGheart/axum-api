<template>
  <n-layout position="absolute" has-sider>
    <!-- 侧栏 -->
    <n-layout-sider
      bordered
      :collapsed="appStore.collapsed"
      collapse-mode="width"
      :collapsed-width="64"
      :width="220"
      :native-scrollbar="false"
      class="layout-sider"
    >
      <!-- Logo -->
      <div class="sider-logo" @click="router.push('/')">
        <span class="logo-icon">⚡</span>
        <span v-show="!appStore.collapsed" class="logo-text">Axum Admin</span>
      </div>

      <!-- 导航菜单 -->
      <n-menu
        :collapsed="appStore.collapsed"
        :collapsed-width="64"
        :collapsed-icon-size="22"
        :options="filteredMenu"
        :value="activeMenu"
        @update:value="onMenuSelect"
      />
    </n-layout-sider>

    <!-- 主区域 -->
    <n-layout>
      <!-- 顶栏 -->
      <n-layout-header bordered class="layout-header">
        <div class="header-left">
          <n-button quaternary size="small" @click="appStore.toggleCollapsed">
            <template #icon>
              <n-icon><MenuIcon /></n-icon>
            </template>
          </n-button>
          <n-breadcrumb class="header-breadcrumb">
            <n-breadcrumb-item>{{ currentTitle }}</n-breadcrumb-item>
          </n-breadcrumb>
        </div>
        <div class="header-right">
          <!-- 主题切换 -->
          <n-button quaternary size="small" @click="appStore.toggleTheme">
            <template #icon>
              <n-icon><SunnyIcon v-if="!appStore.isDark" /><MoonIcon v-else /></n-icon>
            </template>
          </n-button>
          <!-- 用户信息 -->
          <n-dropdown :options="userMenuOptions" @select="onUserMenuSelect">
            <div class="user-info">
              <n-avatar round :size="32" color="#2080f0">
                {{ userInitial }}
              </n-avatar>
              <span v-show="!appStore.collapsed" class="user-name">
                {{ userStore.userInfo?.username || '用户' }}
              </span>
            </div>
          </n-dropdown>
        </div>
      </n-layout-header>

      <!-- 内容区 -->
      <n-layout-content class="layout-content" :native-scrollbar="false">
        <router-view />
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
import { computed, h } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { NIcon } from 'naive-ui'
import {
  MenuOutline as MenuIcon,
  SunnyOutline as SunnyIcon,
  MoonOutline as MoonIcon,
  HomeOutline as HomeIcon,
  SettingsOutline as SettingsIcon,
  LogOutOutline as LogoutIcon,
  PersonOutline as UserIcon,
  ShieldCheckmarkOutline as RoleIcon,
} from '@vicons/ionicons5'
import type { MenuOption } from 'naive-ui'
import { useAppStore } from '@/stores/app'
import { useUserStore } from '@/stores/user'
import { showConfirm } from '@/utils/message'

const router = useRouter()
const route = useRoute()
const appStore = useAppStore()
const userStore = useUserStore()

/** 当前页面标题 */
const currentTitle = computed(() => (route.meta?.title as string) || '首页')

/** 用户头像首字母 */
const userInitial = computed(() => userStore.userInfo?.username?.charAt(0)?.toUpperCase() || 'U')

/** 当前激活的菜单路径 */
const activeMenu = computed(() => route.path)

// ── 导航菜单 ──────────────────────────────────────────────────

function renderIcon(icon: unknown) {
  return () => h(NIcon, null, { default: () => h(icon as never) })
}

const menuOptions: MenuOption[] = [
  {
    label: '首页',
    key: '/',
    icon: renderIcon(HomeIcon),
  },
  {
    label: '系统管理',
    key: '/system',
    icon: renderIcon(SettingsIcon),
    roles: ['admin'],
    children: [
      {
        label: '用户管理',
        key: '/system/user',
        icon: renderIcon(UserIcon),
      },
      {
        label: '角色管理',
        key: '/system/role',
        icon: renderIcon(RoleIcon),
      },
    ],
  },
]

/** 过滤菜单（根据用户角色） */
function filterMenu(options: MenuOption[]): MenuOption[] {
  return options
    .filter((item) => {
      const roles = (item as { roles?: string[] }).roles
      if (!roles || roles.length === 0) return true
      const userRoles =
        userStore.userInfo?.roles || (userStore.userInfo?.role ? [userStore.userInfo.role] : [])
      return roles.some((r) => userRoles.includes(r))
    })
    .map((item) => ({
      ...item,
      children: item.children ? filterMenu(item.children as MenuOption[]) : undefined,
    }))
}

const filteredMenu = computed(() => filterMenu(menuOptions))

function onMenuSelect(key: string) {
  router.push(key)
}

// ── 用户下拉菜单 ──────────────────────────────────────────────

const userMenuOptions = [
  {
    label: '退出登录',
    key: 'logout',
    icon: renderIcon(LogoutIcon),
  },
]

async function onUserMenuSelect(key: string) {
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
  background: var(--bg-card, #fff);
}

.sider-logo {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 16px 20px;
  cursor: pointer;
  border-bottom: 1px solid var(--border-color, #e5e7eb);
}

.logo-icon {
  font-size: 24px;
}

.logo-text {
  font-size: 18px;
  font-weight: 700;
  color: var(--text-primary, #1a1a2e);
  white-space: nowrap;
}

.layout-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 16px;
  height: 52px;
  background: var(--bg-card, #fff);
}

.header-left {
  display: flex;
  align-items: center;
  gap: 12px;
}

.header-breadcrumb {
  font-size: 14px;
}

.header-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.user-info {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 6px;
  transition: background 0.2s;
}

.user-info:hover {
  background: rgba(0, 0, 0, 0.05);
}

.user-name {
  font-size: 14px;
  color: var(--text-primary, #1a1a2e);
}

.layout-content {
  padding: 16px;
  min-height: calc(100vh - 52px);
  background: var(--bg-color, #f5f7fa);
}
</style>
